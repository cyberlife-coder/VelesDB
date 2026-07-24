//! Stable, content-addressed identifier derivation.
//!
//! The Agent Memory SDK keys memories by `u64`; the MCP surface addresses facts
//! by their text content. IDs are derived via FNV-1a 64-bit so the mapping is
//! self-contained and stable regardless of engine internals. Deterministic IDs
//! make `remember` idempotent: re-remembering identical (trimmed) content
//! updates the fact in place.
//!
//! Trade-off: two *distinct* facts whose content hashes to the same value
//! (probability ≈ 2⁻⁶⁴) would coalesce under one id — an accepted property of
//! content-addressing, not a bug to guard against.
//!
//! Delegates to `velesdb_core::wire::stable_hash` (issue #1542) instead of
//! re-declaring the FNV-1a offset/prime constants locally, so this crate's
//! derivation cannot drift from core's canonical implementation. Byte-for-byte
//! output is unchanged from the historical local implementation — see the
//! golden-vector regression test below.

/// Derive a stable `u64` id from arbitrary text via FNV-1a 64-bit.
///
/// Delegates to [`velesdb_core::hash_id`], the canonical cross-engine
/// derivation, so ids produced here agree byte-for-byte with core's.
#[must_use]
pub fn stable_id(text: &str) -> u64 {
    velesdb_core::hash_id(text)
}

/// Derive a stable `u64` id from arbitrary bytes via FNV-1a 64-bit — the
/// same scheme as [`stable_id`], generalized to raw bytes so binary payloads
/// (e.g. decoded media, US-009) can be content-addressed without a lossy
/// round-trip through `String`.
///
/// Delegates to [`velesdb_core::hash_id_bytes`], the exported bytes-level
/// counterpart of [`velesdb_core::hash_id`].
#[must_use]
pub fn stable_id_bytes(bytes: &[u8]) -> u64 {
    velesdb_core::hash_id_bytes(bytes)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn stable_id_bytes_agrees_with_stable_id_on_valid_utf8() {
        assert_eq!(stable_id_bytes("hello".as_bytes()), stable_id("hello"));
    }

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
            assert_eq!(
                stable_id_bytes(input.as_bytes()),
                *expected,
                "stable_id_bytes({input:?}) drifted from its pre-refactor golden vector"
            );
        }
    }
}
