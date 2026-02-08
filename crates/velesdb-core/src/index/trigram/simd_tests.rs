#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::approx_constant
)]
//! Tests for SIMD-optimized trigram operations.
//!
//! Extracted from `simd.rs` for maintainability (04-05 module splitting).

use super::simd::*;
use std::collections::HashSet;

#[test]
fn test_simd_level_detection() {
    let level = TrigramSimdLevel::detect();
    // Should always return a valid level
    assert!(!level.name().is_empty());
}

#[test]
fn test_extract_trigrams_simd_basic() {
    let trigrams = extract_trigrams_simd("hello");
    assert!(!trigrams.is_empty());
    assert!(trigrams.contains(b"hel"));
    assert!(trigrams.contains(b"ell"));
    assert!(trigrams.contains(b"llo"));
}

#[test]
fn test_extract_trigrams_simd_empty() {
    let trigrams = extract_trigrams_simd("");
    assert!(trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_simd_consistency() {
    // SIMD and scalar should produce identical results
    let text = "The quick brown fox jumps over the lazy dog";
    let simd_result = extract_trigrams_simd(text);
    let scalar_result = extract_trigrams_scalar(text);

    assert_eq!(simd_result.len(), scalar_result.len());
    for trigram in &scalar_result {
        assert!(simd_result.contains(trigram));
    }
}

#[test]
fn test_extract_trigrams_simd_long_text() {
    let text = "a".repeat(1000);
    let trigrams = extract_trigrams_simd(&text);
    // Should handle long texts without panic
    assert!(!trigrams.is_empty());
}

#[test]
fn test_count_matching_trigrams() {
    let query: Vec<[u8; 3]> = vec![
        [b'h', b'e', b'l'],
        [b'e', b'l', b'l'],
        [b'l', b'l', b'o'],
        [b'x', b'y', b'z'],
    ];

    let mut doc_set = HashSet::new();
    doc_set.insert([b'h', b'e', b'l']);
    doc_set.insert([b'e', b'l', b'l']);
    doc_set.insert([b'a', b'b', b'c']);

    let count = count_matching_trigrams_simd(&query, &doc_set);
    assert_eq!(count, 2); // 'hel' and 'ell' match
}

#[test]
#[ignore = "Flaky: SIMD perf varies by system load - run manually"]
#[allow(clippy::cast_precision_loss)]
fn test_simd_performance() {
    use std::time::Instant;

    let text = "The quick brown fox jumps over the lazy dog. ".repeat(100);

    let start = Instant::now();
    for _ in 0..1000 {
        let _ = extract_trigrams_simd(&text);
    }
    let simd_time = start.elapsed();

    let start = Instant::now();
    for _ in 0..1000 {
        let _ = extract_trigrams_scalar(&text);
    }
    let scalar_time = start.elapsed();

    println!(
        "SIMD: {:?}, Scalar: {:?}, Speedup: {:.2}x",
        simd_time,
        scalar_time,
        scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64
    );

    // SIMD should not be slower than scalar
    assert!(simd_time <= scalar_time.mul_f32(1.5));
}

// =========================================================================
// Additional tests for coverage
// =========================================================================

#[test]
fn test_extract_trigrams_scalar_empty() {
    let trigrams = extract_trigrams_scalar("");
    assert!(trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_scalar_basic() {
    let trigrams = extract_trigrams_scalar("abc");
    assert!(!trigrams.is_empty());
    // With padding "  abc  ", we get trigrams: "  a", " ab", "abc", "bc ", "c  "
    assert!(trigrams.contains(b"abc"));
}

#[test]
fn test_extract_trigrams_scalar_short() {
    let trigrams = extract_trigrams_scalar("a");
    // With padding "  a  ", we get trigrams: "  a", " a ", "a  "
    assert!(!trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_scalar_two_chars() {
    let trigrams = extract_trigrams_scalar("ab");
    // With padding "  ab  ", we get trigrams
    assert!(!trigrams.is_empty());
}

#[test]
fn test_trigram_simd_level_name() {
    let level = TrigramSimdLevel::Scalar;
    assert_eq!(level.name(), "Scalar");
}

#[test]
fn test_count_matching_trigrams_empty_query() {
    let query: Vec<[u8; 3]> = vec![];
    let doc_set: HashSet<[u8; 3]> = HashSet::new();
    let count = count_matching_trigrams_simd(&query, &doc_set);
    assert_eq!(count, 0);
}

#[test]
fn test_count_matching_trigrams_no_match() {
    let query: Vec<[u8; 3]> = vec![[b'a', b'b', b'c'], [b'd', b'e', b'f']];
    let mut doc_set = HashSet::new();
    doc_set.insert([b'x', b'y', b'z']);
    let count = count_matching_trigrams_simd(&query, &doc_set);
    assert_eq!(count, 0);
}

#[test]
fn test_count_matching_trigrams_all_match() {
    let query: Vec<[u8; 3]> = vec![[b'a', b'b', b'c'], [b'd', b'e', b'f']];
    let mut doc_set = HashSet::new();
    doc_set.insert([b'a', b'b', b'c']);
    doc_set.insert([b'd', b'e', b'f']);
    let count = count_matching_trigrams_simd(&query, &doc_set);
    assert_eq!(count, 2);
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn test_count_matching_trigrams_large_query() {
    // Test with > 16 trigrams to trigger SIMD path
    let query: Vec<[u8; 3]> = (0..20).map(|i| [b'a' + i as u8, b'b', b'c']).collect();
    let mut doc_set = HashSet::new();
    doc_set.insert([b'a', b'b', b'c']);
    doc_set.insert([b'b', b'b', b'c']);
    doc_set.insert([b'c', b'b', b'c']);
    let count = count_matching_trigrams_simd(&query, &doc_set);
    assert_eq!(count, 3);
}

#[test]
fn test_extract_trigrams_unicode() {
    let trigrams = extract_trigrams_simd("héllo");
    assert!(!trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_spaces() {
    let trigrams = extract_trigrams_simd("a b c");
    assert!(!trigrams.is_empty());
}

#[test]
fn test_extract_trigrams_numbers() {
    let trigrams = extract_trigrams_simd("123");
    assert!(trigrams.contains(b"123"));
}
