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

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Derive a stable `u64` id from arbitrary text via FNV-1a 64-bit.
#[must_use]
pub fn stable_id(text: &str) -> u64 {
    text.bytes().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
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
        assert_eq!(stable_id(""), FNV_OFFSET);
    }
}
