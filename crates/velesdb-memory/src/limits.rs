//! Resource caps shared by every adapter (the MCP server and the language
//! bindings).
//!
//! These are security-relevant DoS limits. They live here — not inside any one
//! adapter — so every transport enforces the *same* numbers without a manual
//! "keep in sync" comment, and so a build without the `mcp` feature still sees
//! them. Each adapter formats its own transport-native error; only the values
//! and the clamping policy are shared.

use crate::service::Metadata;

/// Default hop budget for `why` traversal when the caller supplies none.
pub const DEFAULT_WHY_HOPS: usize = 2;

/// Maximum accepted fact size (1 MiB) — prevents allocating huge embeddings.
pub const MAX_FACT_BYTES: usize = 1_048_576;

/// Maximum accepted size of caller-supplied `metadata` (64 KiB), measured as
/// its serialized JSON form. Metadata is a keyed lookup facet (project,
/// author, status, …) — a porte-clés, not a payload — so it gets a much
/// tighter ceiling than [`MAX_FACT_BYTES`]: without one, a caller could smuggle
/// an arbitrarily large JSON blob through `metadata` on every write path
/// (`remember`, `remember_with_ttl`, `remember_extracted`, and each
/// context-compiler fragment's own `metadata`) and force the same unbounded
/// allocation and storage growth the fact-size cap exists to prevent.
pub const MAX_METADATA_BYTES: usize = 64 * 1024;

/// The serialized JSON size of `meta`, in bytes. Returns `usize::MAX` if the
/// map somehow fails to serialize (it never should — `Metadata` is always
/// valid JSON), so a serialization hiccup fails a size check closed rather
/// than silently passing an unmeasured payload.
#[must_use]
pub fn metadata_bytes(meta: &Metadata) -> usize {
    serde_json::to_vec(meta).map_or(usize::MAX, |v| v.len())
}

/// Cap on a `recall` limit — prevents unbounded vector scans (core does not
/// cap `k`, so the adapters do).
pub const MAX_RECALL_LIMIT: usize = 1_000;

/// Cap on `why` hop depth — prevents exponential graph fan-out.
pub const MAX_WHY_HOPS: usize = 10;

/// Maximum accepted size of a single context-compiler fragment (1 MiB, the
/// same ceiling as [`MAX_FACT_BYTES`]) — prevents a single fragment from
/// forcing huge allocations in the compile pipeline.
pub const MAX_FRAGMENT_BYTES: usize = 1_048_576;

/// Cap on the number of fragments in one compile request — bounds the work a
/// single call can demand across every adapter.
pub const MAX_FRAGMENTS: usize = 1_024;

/// Maximum accepted size of a fragment's base64-encoded media payload
/// (US-009, PR1: inline images) — 4 MiB of base64 text, roughly 3 MiB of raw
/// bytes once decoded. Deliberately separate from [`MAX_FRAGMENT_BYTES`],
/// which only ever measures [`crate::context::model::ContextFragment::content`]
/// (the caption): a screenshot is not text, and capping it at the 1 MiB text
/// ceiling would reject ordinary screenshots outright. Measured against
/// `bytes_b64.len()` (the encoded string), so the cap can reject an
/// oversized payload before any base64 decoding is attempted.
pub const MAX_MEDIA_BYTES: usize = 4 * 1024 * 1024;

/// Aggregate cap on ALL media payloads of one request (base64 length,
/// summed). Without it, `MAX_FRAGMENTS` fragments each at [`MAX_MEDIA_BYTES`]
/// would let a single request carry 4 GiB of media — far past the ~1 GiB
/// worst case the text caps allow. 64 MiB comfortably fits a real
/// screenshot-heavy session while bounding decode work.
pub const MAX_TOTAL_MEDIA_BYTES: usize = 64 * 1024 * 1024;

/// Cap on a caller-supplied token budget. A budget cannot force allocations
/// by itself, but an absurd value would make the savings arithmetic
/// meaningless, so adapters clamp to this ceiling instead of erroring.
pub const MAX_TOKEN_BUDGET: u64 = 10_000_000;

/// Clamp a caller-supplied token budget to [`MAX_TOKEN_BUDGET`].
#[must_use]
pub fn clamp_token_budget(budget: u64) -> u64 {
    budget.min(MAX_TOKEN_BUDGET)
}

/// Clamp a caller-supplied recall limit to [`MAX_RECALL_LIMIT`].
#[must_use]
pub fn clamp_recall_limit(k: usize) -> usize {
    k.min(MAX_RECALL_LIMIT)
}

/// Clamp a caller-supplied `why` hop budget to [`MAX_WHY_HOPS`].
#[must_use]
pub fn clamp_hops(hops: usize) -> usize {
    hops.min(MAX_WHY_HOPS)
}
