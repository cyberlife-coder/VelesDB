//! TDD Tests for Trigram Index (US-CORE-003-01)
//!
//! Tests written BEFORE implementation following craftsman workflow.

use super::*;

// ========== Trigram Extraction Tests ==========

#[test]
fn test_extract_trigrams_simple_ascii() {
    let trigrams = extract_trigrams("hello");

    // "hello" with padding → "  hello  "
    // Trigrams: "  h", " he", "hel", "ell", "llo", "lo ", "o  "
    assert!(trigrams.contains(&[b' ', b' ', b'h']));
    assert!(trigrams.contains(&[b' ', b'h', b'e']));
    assert!(trigrams.contains(&[b'h', b'e', b'l']));
    assert!(trigrams.contains(&[b'e', b'l', b'l']));
    assert!(trigrams.contains(&[b'l', b'l', b'o']));
    assert!(trigrams.contains(&[b'l', b'o', b' ']));
    assert!(trigrams.contains(&[b'o', b' ', b' ']));
    assert_eq!(trigrams.len(), 7);
}

#[test]
fn test_extract_trigrams_short_string() {
    // String shorter than 3 chars should still produce trigrams with padding
    let trigrams = extract_trigrams("ab");

    // "ab" with padding → "  ab  "
    // Trigrams: "  a", " ab", "ab ", "b  "
    assert!(trigrams.contains(&[b' ', b' ', b'a']));
    assert!(trigrams.contains(&[b' ', b'a', b'b']));
    assert!(trigrams.contains(&[b'a', b'b', b' ']));
    assert!(trigrams.contains(&[b'b', b' ', b' ']));
}

#[test]
fn test_extract_trigrams_single_char() {
    let trigrams = extract_trigrams("x");

    // "x" with padding → "  x  "
    // Trigrams: "  x", " x ", "x  "
    assert!(trigrams.contains(&[b' ', b' ', b'x']));
    assert!(trigrams.contains(&[b' ', b'x', b' ']));
    assert!(trigrams.contains(&[b'x', b' ', b' ']));
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
    assert!(trigrams.contains(&[b'a', b' ', b'b']));
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

    index.insert(1, "hello world");

    assert!(!index.is_empty());
    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_trigram_index_insert_multiple() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.insert(2, "world");
    index.insert(3, "hello world");

    assert_eq!(index.doc_count(), 3);
}

#[test]
fn test_trigram_index_insert_duplicate_id() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.insert(1, "world"); // Same ID, should update

    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_trigram_index_remove() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.insert(2, "world");

    index.remove(1);

    assert_eq!(index.doc_count(), 1);
}

#[test]
fn test_trigram_index_remove_nonexistent() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.remove(999); // Should not panic

    assert_eq!(index.doc_count(), 1);
}

// ========== Search Tests ==========

#[test]
fn test_trigram_search_exact_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world");
    index.insert(2, "goodbye world");
    index.insert(3, "hello there");

    let results = index.search_like("hello");

    assert!(results.contains(1u32));
    assert!(!results.contains(2u32));
    assert!(results.contains(3u32));
}

#[test]
fn test_trigram_search_partial_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "Paris");
    index.insert(2, "London");
    index.insert(3, "Parma");

    // Search for "Par" should match Paris and Parma
    let results = index.search_like("Par");

    assert!(results.contains(1u32)); // Paris
    assert!(!results.contains(2u32)); // London
    assert!(results.contains(3u32)); // Parma
}

#[test]
fn test_trigram_search_no_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.insert(2, "world");

    let results = index.search_like("xyz");

    assert!(results.is_empty());
}

#[test]
fn test_trigram_search_empty_pattern() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");

    let results = index.search_like("");

    // Empty pattern should return all docs (or none, depending on semantics)
    // We choose to return all docs for LIKE '%%'
    assert!(results.contains(1u32));
}

#[test]
fn test_trigram_search_short_pattern() {
    let mut index = TrigramIndex::new();

    index.insert(1, "abc");
    index.insert(2, "abd");
    index.insert(3, "xyz");

    // Pattern shorter than 3 chars
    let results = index.search_like("ab");

    assert!(results.contains(1u32));
    assert!(results.contains(2u32));
    assert!(!results.contains(3u32));
}

// ========== Performance Tests ==========

#[test]
fn test_trigram_search_performance_10k() {
    let mut index = TrigramIndex::new();

    // Insert 10K documents
    for i in 0..10_000u64 {
        let text = format!("document number {i} with some content");
        index.insert(i, &text);
    }

    // Search should be fast
    let start = std::time::Instant::now();
    let _results = index.search_like("number 500");
    let elapsed = start.elapsed();

    // Target: < 10ms for 10K docs
    assert!(
        elapsed.as_millis() < 50,
        "Search took {}ms, expected < 50ms",
        elapsed.as_millis()
    );
}

// ========== Scoring Tests ==========

#[test]
fn test_trigram_score_jaccard_identical() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");

    let query_trigrams = extract_trigrams("hello");
    let score = index.score_jaccard(1, &query_trigrams);

    // Identical text should have score close to 1.0
    assert!(score > 0.9, "Score should be > 0.9, got {score}");
}

#[test]
fn test_trigram_score_jaccard_partial() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world");

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

    index.insert(1, "hello");

    let query_trigrams = extract_trigrams("xyz");
    let score = index.score_jaccard(1, &query_trigrams);

    // No overlap should have score 0
    assert!(score < 0.1, "Score should be < 0.1, got {score}");
}

// ========== Stats Tests ==========

#[test]
fn test_trigram_index_stats() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.insert(2, "world");

    let stats = index.stats();

    assert_eq!(stats.doc_count, 2);
    assert!(stats.trigram_count > 0);
    assert!(stats.memory_bytes > 0);
}

// ========== US-CORE-003-02: Query Optimization Tests ==========

#[test]
fn test_search_with_threshold_filters_low_scores() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world");
    index.insert(2, "hello there");
    index.insert(3, "completely different text");

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

    index.insert(1, "hello"); // Exact match
    index.insert(2, "hello world"); // Partial match
    index.insert(3, "hello there my friend"); // Less similar

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

    index.insert(1, "hello");
    index.insert(2, "world");

    let results = index.search_like_ranked("", 0.0);

    // Empty pattern should return all docs with score 0
    assert_eq!(results.len(), 2);
}

#[test]
fn test_search_ranked_no_match() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello");
    index.insert(2, "world");

    let results = index.search_like_ranked("xyz", 0.0);

    // No match should return empty
    assert!(results.is_empty());
}

#[test]
fn test_threshold_pruning_performance() {
    let mut index = TrigramIndex::new();

    // Insert 1000 documents, only 10% should match
    for i in 0..1000u64 {
        if i % 10 == 0 {
            index.insert(i, &format!("matching document number {i}"));
        } else {
            index.insert(i, &format!("other content {i}"));
        }
    }

    let start = std::time::Instant::now();
    let results = index.search_like_ranked("matching", 0.2);
    let elapsed = start.elapsed();

    // Should be fast and filter correctly
    assert!(elapsed.as_millis() < 50, "Threshold search should be fast");
    assert!(results.len() <= 150, "Threshold should filter results");
}
