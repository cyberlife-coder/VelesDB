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
// Only `context/media.rs` content-addresses raw bytes, so outside that
// feature this is genuinely unreachable — and `id` is a `pub(crate)`
// module, so `pub` does not make it live. Gating it on its actual
// consumer keeps `-D warnings` honest instead of silencing dead_code.
#[cfg(feature = "context")]
#[must_use]
pub fn stable_id_bytes(bytes: &[u8]) -> u64 {
    velesdb_core::hash_id_bytes(bytes)
}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
