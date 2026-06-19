//! Binary wire formats shared across the server, CLI, and SDKs.
//!
//! These are pure byte (de)serialisers with no storage or persistence
//! dependency, so they compile on every target (including `wasm32`).

pub mod vrb1;

/// Generates a deterministic graph-edge ID from (source, target, label) using
/// FNV-1a over the raw little-endian bytes of `source` and `target` followed by
/// the UTF-8 bytes of `label`.
///
/// This is the canonical edge-id derivation shared across every `VelesDB` engine
/// (core executor, WASM, migrate). Hashing the raw bytes — never a
/// formatted/decimal string with separators — keeps the result stable and
/// identical across crates, so the same logical edge always maps to the same
/// id. Re-inserting that edge without an explicit `id` is therefore idempotent
/// (it overwrites the existing edge); supply explicit `id` values to create
/// multiple edges sharing the same (source, target, label) triple.
///
/// It lives in `wire` (persistence-free, `wasm32`-safe) so binding crates that
/// build core without the `persistence` feature can still delegate to it.
#[must_use]
pub fn hash_edge_id(source: u64, target: u64, label: &str) -> u64 {
    // FNV-1a offset basis and prime for u64
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;

    let mut hash = OFFSET_BASIS;
    for byte in source.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    for byte in target.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    for byte in label.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
