//! Tests for `TrigramFingerprint` bloom filter (Phase 6: Trigram SIMD).

#![allow(clippy::cast_precision_loss)] // Precision loss acceptable in test scoring comparisons

use super::fingerprint::TrigramFingerprint;
use super::index::{extract_trigrams, TrigramIndex};

/// Exact set Jaccard of the trigrams of `a` and `b` — the ground truth the
/// fingerprint estimates.
fn exact_jaccard(a: &str, b: &str) -> f32 {
    let (trigrams_a, trigrams_b) = (extract_trigrams(a), extract_trigrams(b));
    let union = trigrams_a.union(&trigrams_b).count();
    if union == 0 {
        return 0.0;
    }
    trigrams_a.intersection(&trigrams_b).count() as f32 / union as f32
}

/// Largest absolute gap tolerated between the fingerprint estimate and the
/// exact Jaccard on the hand-written pairs below (worst observed: 0.024).
/// The residual is the variance of the bit-count cardinality estimate over a
/// 256-bit width; its bias is checked separately over a generated corpus.
const APPROX_JACCARD_TOLERANCE: f32 = 0.05;

fn assert_approx_tracks_exact(a: &str, b: &str) {
    let (exact, approx) = (exact_jaccard(a, b), approx_jaccard(a, b));
    let diff = (approx - exact).abs();
    assert!(
        diff < APPROX_JACCARD_TOLERANCE,
        "({a}, {b}): approx={approx:.3}, exact={exact:.3}, diff={diff:.3}"
    );
}

/// Fingerprint estimate of the Jaccard of `a` and `b`.
fn approx_jaccard(a: &str, b: &str) -> f32 {
    let (trigrams_a, trigrams_b) = (extract_trigrams(a), extract_trigrams(b));
    TrigramFingerprint::from_trigram_set(&trigrams_a).approx_jaccard(
        &TrigramFingerprint::from_trigram_set(&trigrams_b),
        trigrams_a.len(),
        trigrams_b.len(),
    )
}

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

    // Identical fingerprints: the rescaled union is the exact count ⇒ 1.0.
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

    for (a, b) in pairs {
        assert_approx_tracks_exact(a, b);
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

// ========== Hash Dispersion Tests ==========

#[test]
fn test_trigrams_sharing_first_byte_map_to_distinct_bits() {
    // Regression: `v * C % 256` kept the low byte of the product, which
    // depends on `trigram[0]` alone, so every trigram starting with `a` set
    // the same two bits.
    let fingerprints: std::collections::HashSet<TrigramFingerprint> = (b'a'..=b'z')
        .flat_map(|second| (b'a'..=b'z').map(move |third| [b'a', second, third]))
        .map(|trigram| {
            let mut fp = TrigramFingerprint::default();
            fp.insert(&trigram);
            fp
        })
        .collect();

    // 676 trigrams over C(256, 2) + 256 = 32 896 possible bit sets: a uniform
    // hash yields ~7 expected collisions, so ≥ 650 distinct leaves slack.
    assert!(
        fingerprints.len() >= 650,
        "trigrams `a??` must spread over the fingerprint, got {} distinct bit sets out of 676",
        fingerprints.len()
    );
}

#[test]
fn test_approx_jaccard_tracks_exact_on_short_and_long_documents() {
    let query = "matching";
    let corpus = [
        "matching",
        "matching document number 0",
        "other content 7",
        "pattern matching in compilers",
        "matching 1000 lorem ipsum dolor sit amet consectetur adipiscing elit sed do \
         eiusmod tempor incididunt ut labore et dolore magna aliqua",
        "the quick brown fox jumps over the lazy dog while matching socks are lost \
         in the laundry machine every single week",
    ];

    for doc in corpus {
        assert_approx_tracks_exact(query, doc);
    }
}

// ========== Estimator Accuracy Tests ==========

/// Deterministic xorshift64 stream, so the generated corpus is the same on
/// every run and platform.
struct XorShift(u64);

impl XorShift {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % n as u64).expect("test: index fits usize")
    }
}

const WORDS: [&str; 40] = [
    "the", "of", "and", "to", "in", "is", "that", "for", "it", "as", "was", "with", "be", "by",
    "on", "not", "this", "are", "or", "from", "at", "which", "but", "have", "an", "they", "one",
    "all", "there", "been", "matching", "vector", "index", "search", "database", "query", "graph",
    "lorem", "ipsum", "dolor",
];

fn phrase(rng: &mut XorShift, words: usize) -> String {
    (0..words)
        .map(|_| WORDS[rng.below(WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

/// `(document, query)` pairs over 3–120-word documents: half the queries are
/// a phrase of their document (the LIKE shape), half are unrelated phrases.
fn generated_pairs(count: usize) -> Vec<(String, String)> {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    (0..count)
        .map(|i| {
            let doc_words = [3, 6, 12, 25, 50, 80, 120][rng.below(7)];
            let doc = phrase(&mut rng, doc_words);
            let query = if i % 2 == 0 {
                let words: Vec<&str> = doc.split(' ').collect();
                let start = rng.below(words.len());
                let end = (start + 1 + rng.below(3)).min(words.len());
                words[start..end].join(" ")
            } else {
                let query_words = [3, 6, 12, 25][rng.below(4)];
                phrase(&mut rng, query_words)
            };
            (doc, query)
        })
        .collect()
}

#[test]
fn test_approx_jaccard_is_unbiased_over_a_generated_corpus() {
    // The bit-level ratio `|A AND B| / |A OR B|` overestimated by ~0.1 on
    // average and by up to ~0.4 on long documents: two trigrams of one
    // document sharing a bit shrink `A OR B` but not the shared bits.
    let pairs = generated_pairs(600);
    let errors: Vec<f32> = pairs
        .iter()
        .map(|(doc, query)| approx_jaccard(doc, query) - exact_jaccard(doc, query))
        .collect();

    let n = errors.len() as f32;
    let bias = errors.iter().sum::<f32>() / n;
    let mean_abs = errors.iter().map(|e| e.abs()).sum::<f32>() / n;
    assert!(bias.abs() < 0.02, "estimator bias {bias:.4} over {n} pairs");
    assert!(
        mean_abs < 0.03,
        "mean absolute error {mean_abs:.4} over {n} pairs"
    );
}

#[test]
fn test_approx_jaccard_on_a_saturated_fingerprint_stays_a_ratio() {
    // Over a thousand distinct trigrams set every one of the 256 bits, where
    // the cardinality estimate `-(m/k) ln(1 - X/m)` diverges.
    let long = (0..2_000)
        .map(|i: u32| i.to_string())
        .collect::<Vec<_>>()
        .join("-");
    let trigrams = extract_trigrams(&long);
    let fp = TrigramFingerprint::from_trigram_set(&trigrams);
    assert_eq!(
        fp.approx_intersection_count(&fp),
        256,
        "precondition: saturated"
    );

    let query = extract_trigrams("00ff");
    let score = fp.approx_jaccard(
        &TrigramFingerprint::from_trigram_set(&query),
        trigrams.len(),
        query.len(),
    );
    assert!(
        score.is_finite() && (0.0..=1.0).contains(&score),
        "saturated estimate must stay a ratio, got {score}"
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
fn test_search_like_ranked_filters_and_sorts() {
    let mut index = TrigramIndex::new();

    index.insert(1, "hello world").expect("test: insert");
    index.insert(2, "hello there").expect("test: insert");
    index.insert(3, "goodbye world").expect("test: insert");

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
