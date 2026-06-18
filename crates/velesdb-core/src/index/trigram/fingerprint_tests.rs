//! Tests for `TrigramFingerprint` bloom filter (Phase 6: Trigram SIMD).

#![allow(clippy::cast_precision_loss)] // Precision loss acceptable in test scoring comparisons

use super::fingerprint::TrigramFingerprint;
use super::index::{extract_trigrams, TrigramIndex};

// ========== Construction Tests ==========

#[test]
fn test_empty_fingerprint() {
    let fp = TrigramFingerprint::default();
    assert!(fp.is_empty());
    assert_eq!(fp.approx_intersection_count(&fp), 0);
}

#[test]
fn test_insert_and_bits_set() {
    let mut fp = TrigramFingerprint::default();
    assert!(fp.is_empty());

    fp.insert(b"hel");
    assert!(!fp.is_empty(), "bits should be set after insert");
}

#[test]
fn test_from_text_matches_from_set() {
    let text = "hello world";
    let fp_text = TrigramFingerprint::from_text(text);

    let trigrams = extract_trigrams(text);
    let fp_set = TrigramFingerprint::from_trigram_set(&trigrams);

    assert_eq!(
        fp_text, fp_set,
        "from_text and from_trigram_set must produce identical fingerprints"
    );
}

#[test]
fn test_from_text_empty() {
    let fp = TrigramFingerprint::from_text("");
    assert!(fp.is_empty());
}

// ========== Intersection Tests ==========

#[test]
fn test_approx_intersection_identical() {
    let fp = TrigramFingerprint::from_text("hello world");
    let count = fp.approx_intersection_count(&fp);

    // Self-intersection should return all set bits.
    assert!(count > 0, "self-intersection must be positive");
}

#[test]
fn test_approx_intersection_disjoint() {
    // Two strings with no shared trigrams.
    let fp_a = TrigramFingerprint::from_text("xyz");
    let fp_b = TrigramFingerprint::from_text("qwu");

    let trigrams_a = extract_trigrams("xyz");
    let trigrams_b = extract_trigrams("qwu");

    // Verify exact disjointness first.
    let exact_intersection = trigrams_a.intersection(&trigrams_b).count();

    // If exact intersection is zero, fingerprint intersection should be
    // very small (bloom collisions are possible but rare).
    if exact_intersection == 0 {
        let approx = fp_a.approx_intersection_count(&fp_b);
        // Allow a small number of collisions (bloom FPR).
        assert!(
            approx <= 4,
            "disjoint strings should have near-zero intersection, got {approx}"
        );
    }
}

#[test]
fn test_approx_intersection_subset() {
    // "hel" is a substring of "hello", so trigrams overlap.
    let fp_full = TrigramFingerprint::from_text("hello");
    let fp_part = TrigramFingerprint::from_text("hel");

    let count = fp_part.approx_intersection_count(&fp_full);
    assert!(count > 0, "overlapping texts must share bits");
}

// ========== Jaccard Tests ==========

#[test]
fn test_approx_jaccard_orders_by_similarity() {
    let fp = |t: &str| TrigramFingerprint::from_text(t);
    let cnt = |t: &str| extract_trigrams(t).len();

    let base = "hello world";
    let base_fp = fp(base);
    let base_cnt = cnt(base);

    // Identical fingerprints: AND == OR == A ⇒ ratio is exactly 1.0.
    let j_same = base_fp.approx_jaccard(&fp(base), base_cnt, cnt(base));
    assert!(j_same > 0.9, "identical text must score ~1.0, got {j_same}");

    // Shares the whole "hello " prefix.
    let j_close = base_fp.approx_jaccard(&fp("hello there"), base_cnt, cnt("hello there"));
    // Shares essentially nothing with "hello world".
    let j_far = base_fp.approx_jaccard(
        &fp("completely different"),
        base_cnt,
        cnt("completely different"),
    );

    assert!(
        j_close > j_far,
        "more-similar text must score strictly higher: close={j_close}, far={j_far}"
    );
    // Still cheap and safe to keep the bounds as an invariant guard.
    assert!((0.0..=1.0).contains(&j_close) && (0.0..=1.0).contains(&j_far));
}

#[test]
fn test_approx_jaccard_identical_text() {
    let text = "hello world";
    let fp = TrigramFingerprint::from_text(text);
    let count = extract_trigrams(text).len();

    let jaccard = fp.approx_jaccard(&fp, count, count);
    assert!(
        (jaccard - 1.0).abs() < f32::EPSILON,
        "identical text self-Jaccard must be exactly 1.0, got {jaccard}"
    );
}

#[test]
fn test_approx_jaccard_vs_exact() {
    // Use longer texts where the 256-bit bloom Jaccard estimator has
    // better signal-to-noise ratio (more trigrams → lower collision rate).
    let pairs = [
        (
            "the quick brown fox jumps over the lazy dog near the river",
            "the quick brown fox leaps over the lazy cat near the river",
        ),
        (
            "information retrieval systems use inverted indexes for speed",
            "information retrieval engines use inverted indexes for speed",
        ),
        (
            "vector database with approximate nearest neighbor search algorithms",
            "vector database with exact nearest neighbor search algorithms",
        ),
    ];

    for (a, b) in &pairs {
        let trigrams_a = extract_trigrams(a);
        let trigrams_b = extract_trigrams(b);

        // Exact Jaccard.
        let exact_inter = trigrams_a.intersection(&trigrams_b).count();
        let exact_union = trigrams_a.union(&trigrams_b).count();
        let exact_jaccard = if exact_union == 0 {
            0.0
        } else {
            exact_inter as f32 / exact_union as f32
        };

        // Approximate Jaccard.
        let fp_a = TrigramFingerprint::from_trigram_set(&trigrams_a);
        let fp_b = TrigramFingerprint::from_trigram_set(&trigrams_b);
        let approx_jaccard = fp_a.approx_jaccard(&fp_b, trigrams_a.len(), trigrams_b.len());

        // 256-bit bloom Jaccard consistently overestimates due to hash
        // collisions. For texts with 40+ trigrams the error is typically
        // under 20%. We allow 25% to account for edge cases.
        let diff = (approx_jaccard - exact_jaccard).abs();
        assert!(
            diff < 0.25,
            "({a}, {b}): approx={approx_jaccard:.3}, exact={exact_jaccard:.3}, diff={diff:.3}"
        );
    }
}

#[test]
fn test_approx_jaccard_zero_counts() {
    let fp = TrigramFingerprint::default();
    let jaccard = fp.approx_jaccard(&fp, 0, 0);
    assert!(
        (jaccard - 0.0).abs() < f32::EPSILON,
        "zero-count Jaccard should be 0.0"
    );
}

// ========== Integration Tests ==========

#[test]
fn test_fingerprint_integration_with_index() {
    let mut index = TrigramIndex::new();

    index
        .insert(1, "machine learning algorithms")
        .expect("test: insert");
    index
        .insert(2, "machine learning models")
        .expect("test: insert");
    index
        .insert(3, "database indexing strategies")
        .expect("test: insert");
    index
        .insert(4, "completely unrelated topic")
        .expect("test: insert");

    // score_jaccard_fast should rank "machine learning" docs higher.
    let query = "machine learning";
    let query_trigrams = extract_trigrams(query);
    let query_fp = TrigramFingerprint::from_trigram_set(&query_trigrams);

    let score_1 = index.score_jaccard_fast(1, &query_fp, query_trigrams.len());
    let score_2 = index.score_jaccard_fast(2, &query_fp, query_trigrams.len());
    let score_3 = index.score_jaccard_fast(3, &query_fp, query_trigrams.len());
    let score_4 = index.score_jaccard_fast(4, &query_fp, query_trigrams.len());

    // Docs 1 and 2 should score higher than docs 3 and 4.
    assert!(
        score_1 > score_3,
        "doc 1 ({score_1}) should score higher than doc 3 ({score_3})"
    );
    assert!(
        score_2 > score_4,
        "doc 2 ({score_2}) should score higher than doc 4 ({score_4})"
    );
}

#[test]
fn test_search_like_ranked_uses_fingerprints() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");
    index.insert(2, "hello there").expect("test: insert");
    index.insert(3, "goodbye world").expect("test: insert");

    // search_like_ranked internally uses fingerprints now.
    let results = index.search_like_ranked("hello", 0.1);

    // Docs 1 and 2 contain "hello", doc 3 does not.
    assert!(results.iter().any(|(id, _)| *id == 1));
    assert!(results.iter().any(|(id, _)| *id == 2));
    assert!(!results.iter().any(|(id, _)| *id == 3));

    // Results must be sorted descending by score.
    for window in results.windows(2) {
        assert!(window[0].1 >= window[1].1);
    }
}

#[test]
fn test_fingerprint_survives_remove() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");
    index.insert(2, "goodbye world").expect("test: insert");

    // Remove doc 1 — its fingerprint should be gone.
    index.remove(1);

    let query_trigrams = extract_trigrams("hello");
    let query_fp = TrigramFingerprint::from_trigram_set(&query_trigrams);

    // score_jaccard_fast on removed doc should return 0.0.
    let score = index.score_jaccard_fast(1, &query_fp, query_trigrams.len());
    assert!(
        (score - 0.0).abs() < f32::EPSILON,
        "removed doc should score 0.0, got {score}"
    );
}

#[test]
fn test_fingerprint_update_on_reinsert() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");
    index.insert(1, "goodbye world").expect("test: insert"); // Re-insert same ID with different text.

    let query_trigrams = extract_trigrams("goodbye");
    let query_fp = TrigramFingerprint::from_trigram_set(&query_trigrams);

    let score = index.score_jaccard_fast(1, &query_fp, query_trigrams.len());
    assert!(
        score > 0.0,
        "re-inserted doc should match new text, got {score}"
    );
}
