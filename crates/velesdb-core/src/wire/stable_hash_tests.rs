//! Reference-vector unit tests for the stable cross-engine ID hashing API.
//!
//! Feature: core-control-plane-boundary, Task 3.3.
//! These assert `hash_id` / `hash_edge_id` against frozen, hand-verified
//! FNV-1a reference outputs. They live in their own module (separate from the
//! property tests in `stable_hash_property_tests`) so the fixed golden vectors
//! stay clearly delineated from the generative coverage.
//!
//! The `hash_id` vectors for `""`, `"a"`, and `"foobar"` are the canonical
//! published FNV-1a 64-bit test vectors, so a change to any of these constants
//! signals a break in cross-engine ID compatibility.

use super::stable_hash::{hash_edge_id, hash_id, hash_id_bytes, FNV_OFFSET_BASIS};

// --- hash_id_bytes reference vectors (issue #1542) ---
//
// `hash_id_bytes` is the exported bytes-level fold that `velesdb-memory` and
// `velesdb-migrate` now delegate to instead of re-declaring their own
// FNV-1a constants. It must agree with `hash_id` on every `&str`'s UTF-8
// bytes, and must hash multi-byte UTF-8 sequences over raw bytes (not code
// points).

#[test]
fn test_hash_id_bytes_agrees_with_hash_id_on_ascii() {
    assert_eq!(hash_id_bytes(b""), hash_id(""));
    assert_eq!(hash_id_bytes(b"a"), hash_id("a"));
    assert_eq!(hash_id_bytes(b"tenant:acme"), hash_id("tenant:acme"));
}

#[test]
fn test_hash_id_bytes_agrees_with_hash_id_on_multi_byte_utf8() {
    // 2-byte (é), 3-byte (CJK), and 4-byte (emoji) UTF-8 sequences.
    for input in ["café", "日本語", "emoji:🚀", "mixed-Ünïcödé-42"] {
        assert_eq!(
            hash_id_bytes(input.as_bytes()),
            hash_id(input),
            "hash_id_bytes/hash_id disagree for {input:?}"
        );
    }
}

#[test]
fn test_hash_id_bytes_hashes_non_utf8_bytes() {
    let bytes = [0xFFu8, 0x00, 0x89, 0x50, 0x4E, 0x47];
    assert_eq!(hash_id_bytes(&bytes), hash_id_bytes(&bytes));
    assert_ne!(hash_id_bytes(&bytes), hash_id_bytes(&bytes[1..]));
}

// --- hash_id reference vectors (Requirement 4.2) ---

#[test]
fn test_hash_id_empty_equals_offset_basis() {
    // The empty byte sequence performs no fold steps, so `hash_id("")` is
    // exactly the FNV-1a offset basis.
    assert_eq!(hash_id(""), FNV_OFFSET_BASIS);
    assert_eq!(hash_id(""), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn test_hash_id_single_char_matches_reference() {
    // Canonical published FNV-1a-64 vector for "a".
    assert_eq!(hash_id("a"), 0xaf63_dc4c_8601_ec8c);
}

#[test]
fn test_hash_id_word_matches_reference() {
    // Canonical published FNV-1a-64 vector for "foobar".
    assert_eq!(hash_id("foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn test_hash_id_prefixed_key_matches_reference() {
    // A representative tenant-scoped identifier as used by consumers.
    assert_eq!(hash_id("tenant:acme"), 0x434a_088f_8b77_5207);
}

// --- hash_edge_id reference vectors (Requirement 4.3) ---

#[test]
fn test_hash_edge_id_zero_triple_matches_reference() {
    // (0, 0, "") folds the offset basis over 16 zero bytes and no label bytes.
    assert_eq!(hash_edge_id(0, 0, ""), 0x8820_1fb9_60ff_6465);
}

#[test]
fn test_hash_edge_id_labeled_edge_matches_reference() {
    assert_eq!(hash_edge_id(1, 2, "knows"), 0x083a_4358_f694_89c6);
}

#[test]
fn test_hash_edge_id_second_labeled_edge_matches_reference() {
    assert_eq!(hash_edge_id(42, 7, "follows"), 0xca14_ae35_d9c6_9a62);
}

// --- structural guards distinguishing the derivations ---

#[test]
fn test_hash_edge_id_is_order_sensitive() {
    // Swapping source and target must change the derived edge id: the raw
    // little-endian bytes are folded positionally, not commutatively.
    assert_ne!(hash_edge_id(1, 2, "knows"), hash_edge_id(2, 1, "knows"));
}

#[test]
fn test_hash_edge_id_label_affects_output() {
    // The label participates in the fold, so distinct labels over the same
    // endpoints yield distinct ids.
    assert_ne!(hash_edge_id(1, 2, "knows"), hash_edge_id(1, 2, "likes"));
}
