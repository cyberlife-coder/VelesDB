//! Stable, cross-engine ID hashing (FNV-1a over 64-bit).
//!
//! This module is the single authoritative source for `VelesDB`'s stable
//! string→`u64` derivation. It lives in `wire` (persistence-free, `wasm32`-safe)
//! so every binding crate — including targets built without the `persistence`
//! feature (WASM) — can delegate to it.
//!
//! # Why a dedicated stable hasher
//!
//! Consumers that persist a numeric ID derived from a string, or that exchange
//! such an ID between processes/nodes/engines, MUST use [`hash_id`]. They MUST
//! NOT use `std::collections::hash_map::DefaultHasher` (or any other
//! std-library hasher): `DefaultHasher` is randomly seeded per process and is
//! explicitly documented as not guaranteed to be stable across runs or
//! versions, so IDs derived from it diverge across processes, restarts, and
//! platforms. [`hash_id`] uses FNV-1a, which produces identical output for
//! identical input across processes, runs, and architectures.
//!
//! The FNV-1a offset basis and prime are exposed ([`FNV_OFFSET_BASIS`],
//! [`FNV_PRIME`]) so the algorithm is documented, auditable, and reproducible
//! by any other implementation that must agree byte-for-byte with core.

/// FNV-1a 64-bit offset basis (the initial hash accumulator).
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime multiplier.
pub const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// Folds `bytes` into `hash` using the FNV-1a step (XOR then multiply).
///
/// The shared core for every stable-hash derivation in this module, so all
/// derivations produce byte-identical output for identical byte sequences.
#[inline]
#[must_use]
fn fnv1a_fold(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Stable, platform-independent string → `u64` hash (FNV-1a over UTF-8 bytes).
///
/// Produces identical output for identical input across processes, runs, and
/// architectures. Use this — never a std-library hasher such as
/// `DefaultHasher` — for any persisted or cross-engine-interoperable numeric
/// ID.
///
/// # Examples
///
/// ```
/// use velesdb_core::hash_id;
///
/// // Deterministic: the same input always yields the same u64, in any process.
/// assert_eq!(hash_id("tenant:acme"), hash_id("tenant:acme"));
/// // The empty string hashes to the FNV-1a offset basis.
/// assert_eq!(hash_id(""), 0xcbf2_9ce4_8422_2325);
/// ```
#[must_use]
pub fn hash_id(input: &str) -> u64 {
    fnv1a_fold(FNV_OFFSET_BASIS, input.as_bytes())
}

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
/// Built on the same FNV-1a core as [`hash_id`]; the byte-level output is
/// unchanged from the historical derivation, so existing persisted edge IDs
/// remain valid.
///
/// # Examples
///
/// ```
/// use velesdb_core::hash_edge_id;
///
/// // Deterministic: the same (source, target, label) triple always maps to
/// // the same edge id, in any process or on any architecture.
/// assert_eq!(hash_edge_id(1, 2, "knows"), hash_edge_id(1, 2, "knows"));
/// // The derivation is order-sensitive: swapping endpoints yields a
/// // different edge id (a directed edge, not an undirected pair).
/// assert_ne!(hash_edge_id(1, 2, "knows"), hash_edge_id(2, 1, "knows"));
/// ```
#[must_use]
pub fn hash_edge_id(source: u64, target: u64, label: &str) -> u64 {
    let hash = fnv1a_fold(FNV_OFFSET_BASIS, &source.to_le_bytes());
    let hash = fnv1a_fold(hash, &target.to_le_bytes());
    fnv1a_fold(hash, label.as_bytes())
}
