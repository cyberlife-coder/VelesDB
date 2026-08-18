use super::*;

#[test]
fn same_input_yields_same_id() {
    assert_eq!(stable_id("hello"), stable_id("hello"));
}

#[test]
fn different_inputs_yield_different_ids() {
    assert_ne!(stable_id("hello"), stable_id("world"));
}

#[test]
fn empty_string_yields_offset_basis() {
    // Core's FNV-1a offset basis, re-asserted here as a literal (rather
    // than importing a private core constant) so this test still pins
    // the historical value if the delegation ever changes.
    assert_eq!(stable_id(""), 0xcbf2_9ce4_8422_2325);
}

#[cfg(feature = "context")]
#[test]
fn stable_id_bytes_agrees_with_stable_id_on_valid_utf8() {
    assert_eq!(stable_id_bytes("hello".as_bytes()), stable_id("hello"));
}

#[cfg(feature = "context")]
#[test]
fn stable_id_bytes_hashes_non_utf8_bytes() {
    let bytes = [0xFFu8, 0x00, 0x89, 0x50, 0x4E, 0x47];
    assert_eq!(stable_id_bytes(&bytes), stable_id_bytes(&bytes));
    assert_ne!(stable_id_bytes(&bytes), stable_id_bytes(&bytes[1..]));
}

// ─────────────────────────────────────────────────────────────
// Issue #1542: golden vectors for `stable_id`/`stable_id_bytes`,
// captured against the pre-refactor local FNV-1a implementation.
// `id.rs` is about to stop re-declaring FNV_OFFSET/FNV_PRIME and
// delegate to `velesdb_core::hash_id`/`hash_id_bytes` instead; these
// values must stay byte-identical after that change, otherwise every
// previously-remembered fact's id (and therefore its idempotent
// re-remember behavior) would silently change.
// ─────────────────────────────────────────────────────────────
#[test]
fn stable_id_golden_vectors_unchanged_by_delegation() {
    let vectors: &[(&str, u64)] = &[
        ("", 0xcbf2_9ce4_8422_2325),
        ("a", 0xaf63_dc4c_8601_ec8c),
        ("hello", 0xa430_d846_80aa_bd0b),
        ("world", 0x4f59_ff5e_730c_8af3),
        ("tenant:acme", 0x434a_088f_8b77_5207),
        // Multi-byte UTF-8: 2-byte (é), 3-byte (CJK), and 4-byte (emoji)
        // sequences must hash over raw bytes, not code points.
        ("café", 0x48e8_823a_cfa4_0d89),
        ("日本語", 0xee9e_e2b5_c854_ef87),
        ("emoji:🚀", 0x5063_383e_8fb5_57fa),
        ("mixed-Ünïcödé-42", 0x3019_47e7_0a3d_8809),
        ("fact:the sky is blue", 0x5ff1_6ac5_c3bf_e13b),
    ];

    for (input, expected) in vectors {
        assert_eq!(
            stable_id(input),
            *expected,
            "stable_id({input:?}) drifted from its pre-refactor golden vector"
        );
        // `stable_id_bytes` only exists under `context` (see its own
        // gate above), so the assertion must carry the same gate — the
        // golden vectors for `stable_id` are checked either way.
        #[cfg(feature = "context")]
        assert_eq!(
            stable_id_bytes(input.as_bytes()),
            *expected,
            "stable_id_bytes({input:?}) drifted from its pre-refactor golden vector"
        );
    }
}
