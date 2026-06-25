//! Stable, content-addressed identifier derivation.
//!
//! The Agent Memory SDK keys memories by `u64`; the MCP surface addresses facts
//! by their text content. We derive the id from the engine's own canonical
//! FNV-1a hash (`velesdb_core::hash_edge_id`) so the wrapper never carries a
//! duplicate copy of the hash constants. Deterministic ids make `remember`
//! idempotent: re-remembering identical content updates the fact in place.
//!
//! Trade-off: two *distinct* facts whose content hashes to the same value
//! (probability ≈ 2⁻⁶⁴) would coalesce under one id — an accepted property of
//! content-addressing, not a bug to guard against.

/// Derive a stable `u64` id from arbitrary text, via the engine's FNV-1a hash.
#[must_use]
pub fn stable_id(text: &str) -> u64 {
    velesdb_core::hash_edge_id(0, 0, text)
}
