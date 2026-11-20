//! TDD Tests for Trigram Index (US-CORE-003-01)
//!
//! Tests written BEFORE implementation following craftsman workflow.

use super::index::extract_trigrams_for_pattern;
use super::*;

// ========== Trigram Extraction Tests ==========

#[test]
fn test_extract_trigrams_simple_ascii() {
    let trigrams = extract_trigrams("hello");

    // "hello" with padding → "  hello  "
    // Trigrams: "  h", " he", "hel", "ell", "llo", "lo ", "o  "
    assert!(trigrams.contains(b"  h"));
    assert!(trigrams.contains(b" he"));
    assert!(trigrams.contains(b"hel"));
    assert!(trigrams.contains(b"ell"));
    assert!(trigrams.contains(b"llo"));
    assert!(trigrams.contains(b"lo "));
    assert!(trigrams.contains(b"o  "));
    assert_eq!(trigrams.len(), 7);
}

#[test]
fn test_extract_trigrams_short_string() {
    // String shorter than 3 chars should still produce trigrams with padding
    let trigrams = extract_trigrams("ab");

    // "ab" with padding → "  ab  "
    // Trigrams: "  a", " ab", "ab ", "b  "
    assert!(trigrams.contains(b"  a"));
    assert!(trigrams.contains(b" ab"));
    assert!(trigrams.contains(b"ab "));
    assert!(trigrams.contains(b"b  "));
}

#[test]
fn test_extract_trigrams_single_char() {
    let trigrams = extract_trigrams("x");

    // "x" with padding → "  x  "
    // Trigrams: "  x", " x ", "x  "
    assert!(trigrams.contains(b"  x"));
    assert!(trigrams.contains(b" x "));
    assert!(trigrams.contains(b"x  "));
}

#[test]
fn test_extract_trigrams_empty_string() {
    let trigrams = extract_trigrams("");
    assert!(trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_with_spaces() {
    let trigrams = extract_trigrams("a b");

    // Should handle internal spaces
    assert!(trigrams.contains(b"a b"));
}

#[test]
fn test_extract_trigrams_utf8_accents() {
    // UTF-8 aware: accented characters
    let trigrams = extract_trigrams("café");

    // Should handle UTF-8 properly (byte-level trigrams)
    assert!(!trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_case_preserved() {
    let lower = extract_trigrams("abc");
    let upper = extract_trigrams("ABC");

    // Trigrams should be different for different cases
    assert_ne!(lower, upper);
}

// ========== TrigramIndex CRUD Tests ==========

#[test]
fn test_trigram_index_new() {
    let index = TrigramIndex::new();
    assert!(index.is_empty());
    assert_eq!(index.doc_count(), 0);
}

#[test]
fn test_trigram_index_insert_single() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");

    assert!(!index.is_empty());
    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_trigram_index_insert_multiple() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(2, "world").expect("test: insert");
    index.insert(3, "hello world").expect("test: insert");

    assert_eq!(index.doc_count(), 3);
}

#[test]
fn test_trigram_index_insert_duplicate_id() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(1, "world").expect("test: insert"); // Same ID, should update

    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_trigram_index_remove() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(2, "world").expect("test: insert");

    index.remove(1);

    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_trigram_index_remove_nonexistent() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.remove(999); // Should not panic

    assert_eq!(index.doc_count(), 1);
}

// ========== Search Tests ==========

#[test]
fn test_trigram_search_exact_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");
    index.insert(2, "goodbye world").expect("test: insert");
    index.insert(3, "hello there").expect("test: insert");

    let results = index.search_like("hello");

    assert!(results.contains(1u32));
    assert!(!results.contains(2u32));
    assert!(results.contains(3u32));
}

#[test]
fn test_trigram_search_partial_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "Paris").expect("test: insert");
    index.insert(2, "London").expect("test: insert");
    index.insert(3, "Parma").expect("test: insert");

    // Search for "Par" should match Paris and Parma
    let results = index.search_like("Par");

    assert!(results.contains(1u32)); // Paris
    assert!(!results.contains(2u32)); // London
    assert!(results.contains(3u32)); // Parma
}

#[test]
fn test_trigram_search_no_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(2, "world").expect("test: insert");

    let results = index.search_like("xyz");

    assert!(results.is_empty());
}

#[test]
fn test_trigram_search_empty_pattern() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");

    let results = index.search_like("");

    // Empty pattern should return all docs (or none, depending on semantics)
    // We choose to return all docs for LIKE '%%'
    assert!(results.contains(1u32));
}

#[test]
fn test_trigram_search_short_pattern() {
    let mut index = TrigramIndex::new();

    index.insert(1, "abc").expect("test: insert");
    index.insert(2, "abd").expect("test: insert");
    index.insert(3, "xyz").expect("test: insert");

    // Pattern shorter than 3 chars
    let results = index.search_like("ab");

    assert!(results.contains(1u32));
    assert!(results.contains(2u32));
    assert!(!results.contains(3u32));
}

// ========== Pruning Tests ==========

/// What makes `search_like` fast is that the trigram intersection prunes the
/// corpus before any verification — so that is what is asserted, as a count,
/// not as a wall-clock bound that measures the runner.
///
/// The pattern is a prefix: pattern trigrams carry the leading padding, so
/// `search_like` filters for `LIKE 'pattern%'`. The timed version of this test
/// searched `"number 500"`, matched nothing, and measured an empty search.
#[test]
fn test_trigram_search_prunes_10k_corpus_without_false_negatives() {
    const PREFIX: &str = "document number 500";

    let mut index = TrigramIndex::new();
    let docs: Vec<(u64, String)> = (0..10_000u64)
        .map(|i| (i, format!("document number {i} with some content")))
        .collect();
    for (id, text) in &docs {
        index.insert(*id, text).expect("test: insert");
    }

    let candidates = index.search_like(PREFIX);

    let exact: Vec<u64> = docs
        .iter()
        .filter(|(_, text)| text.starts_with(PREFIX))
        .map(|(id, _)| *id)
        .collect();
    assert!(!exact.is_empty(), "the corpus holds matches to find");
    for id in &exact {
        let id = u32::try_from(*id).expect("test: ids fit u32");
        assert!(candidates.contains(id), "candidate filter dropped {id}");
    }
    assert!(
        candidates.len() <= 100,
        "intersection should prune 99% of 10K docs, kept {}",
        candidates.len()
    );
}

// ========== Scoring Tests ==========

#[test]
fn test_trigram_score_jaccard_identical() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");

    let query_trigrams = extract_trigrams("hello");
    let score = index.score_jaccard(1, &query_trigrams);

    // Identical text should have score close to 1.0
    assert!(score > 0.9, "Score should be > 0.9, got {score}");
}

#[test]
fn test_trigram_score_jaccard_partial() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");

    let query_trigrams = extract_trigrams("hello");
    let score = index.score_jaccard(1, &query_trigrams);

    // Partial match should have score between 0 and 1
    assert!(
        score > 0.0 && score < 1.0,
        "Score should be between 0 and 1, got {score}"
    );
}

#[test]
fn test_trigram_score_jaccard_no_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");

    let query_trigrams = extract_trigrams("xyz");
    let score = index.score_jaccard(1, &query_trigrams);

    // No overlap should have score 0
    assert!(score < 0.1, "Score should be < 0.1, got {score}");
}

#[test]
fn test_search_like_ranked_scores_are_exact_jaccard() {
    // A candidate holds every trigram of the pattern, so its Jaccard is
    // exactly |pattern trigrams| / |doc trigrams|: ranking needs no estimate.
    // This test is the live guard of that invariant (`candidate_jaccard`).
    let mut index = TrigramIndex::new();
    index.insert(1, "matching").expect("test: insert");
    index
        .insert(2, "matching document number 0")
        .expect("test: insert");
    index
        .insert(
            3,
            "matching 1000 lorem ipsum dolor sit amet consectetur adipiscing elit sed do \
             eiusmod tempor incididunt ut labore et dolore magna aliqua",
        )
        .expect("test: insert");
    index
        .insert(4, "matching patterns in compilers")
        .expect("test: insert");

    let pattern = "matching";
    let query_trigrams = extract_trigrams_for_pattern(pattern);
    let results = index.search_like_ranked(pattern, 0.0);

    assert_eq!(results.len(), 4, "every doc starts with the pattern");
    for (id, score) in results {
        let exact = index.score_jaccard(id, &query_trigrams);
        assert!(
            (score - exact).abs() < f32::EPSILON,
            "doc {id}: ranked score {score} must equal the exact Jaccard {exact}"
        );
    }
}

// ========== Stats Tests ==========

#[test]
fn test_trigram_index_stats() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(2, "world").expect("test: insert");

    let stats = index.stats();

    assert_eq!(stats.doc_count, 2);
    assert!(stats.trigram_count > 0);
    assert!(stats.memory_bytes > 0);
}

// ========== US-CORE-003-02: Query Optimization Tests ==========

#[test]
fn test_search_with_threshold_filters_low_scores() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");
    index.insert(2, "hello there").expect("test: insert");
    index
        .insert(3, "completely different text")
        .expect("test: insert");

    // Search with threshold should filter out low-scoring docs
    // Use low threshold (0.1) to include partial matches
    let results = index.search_like_ranked("hello", 0.1);

    // Doc 3 should be filtered out (no "hello" trigrams)
    assert!(!results.iter().any(|(id, _)| *id == 3));
    // Docs 1 and 2 should be included (contain "hello")
    assert!(results.iter().any(|(id, _)| *id == 1));
    assert!(results.iter().any(|(id, _)| *id == 2));
}

#[test]
fn test_search_ranked_returns_sorted_by_score() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert"); // Exact match
    index.insert(2, "hello world").expect("test: insert"); // Partial match
    index
        .insert(3, "hello there my friend")
        .expect("test: insert"); // Less similar

    let results = index.search_like_ranked("hello", 0.0);

    // Results should be sorted by score descending
    assert!(results.len() >= 2);
    for window in results.windows(2) {
        assert!(
            window[0].1 >= window[1].1,
            "Results should be sorted by score descending"
        );
    }
}

#[test]
fn test_search_ranked_empty_pattern() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(2, "world").expect("test: insert");

    let results = index.search_like_ranked("", 0.0);

    // Empty pattern should return all docs with score 0
    assert_eq!(results.len(), 2);
}

#[test]
fn test_search_ranked_no_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello").expect("test: insert");
    index.insert(2, "world").expect("test: insert");

    let results = index.search_like_ranked("xyz", 0.0);

    // No match should return empty
    assert!(results.is_empty());
}

#[test]
fn test_threshold_pruning_keeps_only_matching_docs() {
    let mut index = TrigramIndex::new();

    // Insert 1000 documents, only 10% should match
    for i in 0..1000u64 {
        if i % 10 == 0 {
            index
                .insert(i, &format!("matching document number {i}"))
                .expect("test: insert");
        } else {
            index
                .insert(i, &format!("other content {i}"))
                .expect("test: insert");
        }
    }

    // Candidates the threshold, not the trigram intersection, must drop:
    // they hold the pattern among so many other characters that their
    // Jaccard is low, exact and fingerprint-approximated alike.
    let diluted: Vec<u64> = (1_000..1_010).collect();
    for &id in &diluted {
        let text = format!(
            "matching {id} ABCDEFGHIJKLMNOPQRSTUVWXYZ bdefjklopqrsuvwxyz !#$%&*+-/:;<=>?@[]^_{{|}}~"
        );
        index.insert(id, &text).expect("test: insert");
    }

    let candidates = index.search_like("matching");
    let unthresholded = index.search_like_ranked("matching", 0.0);
    let results = index.search_like_ranked("matching", 0.2);

    // Positive control: without a threshold the diluted docs are ranked, so
    // their absence below is the threshold's doing.
    for &id in &diluted {
        let id32 = u32::try_from(id).expect("test: ids fit u32");
        assert!(candidates.contains(id32), "doc {id} is a trigram candidate");
        assert!(
            unthresholded.iter().any(|(r, _)| *r == id),
            "doc {id} is ranked when no threshold applies"
        );
        assert!(
            results.iter().all(|(r, _)| *r != id),
            "doc {id} should fall under the threshold"
        );
    }

    // The threshold removes exactly the sub-threshold scores, nothing else
    // (compared by id: ties are ordered by an unstable sort).
    let by_id = |mut v: Vec<(u64, f32)>| {
        v.sort_unstable_by_key(|(id, _)| *id);
        v
    };
    let expected: Vec<(u64, f32)> = unthresholded
        .iter()
        .copied()
        .filter(|(_, score)| *score >= 0.2)
        .collect();
    assert_eq!(by_id(results.clone()), by_id(expected));
    assert!(!results.is_empty(), "matching docs clear the threshold");
    for (id, score) in &results {
        assert_eq!(id % 10, 0, "doc {id} is not a well-matching doc");
        assert!(*score >= 0.2, "doc {id} scored {score} under the threshold");
    }
}
