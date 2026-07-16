//! Resource caps shared by every adapter (the MCP server and the language
//! bindings).
//!
//! These are security-relevant DoS limits. They live here — not inside any one
//! adapter — so every transport enforces the *same* numbers without a manual
//! "keep in sync" comment, and so a build without the `mcp` feature still sees
//! them. Each adapter formats its own transport-native error; only the values
//! and the clamping policy are shared.

/// Default hop budget for `why` traversal when the caller supplies none.
pub const DEFAULT_WHY_HOPS: usize = 2;

/// Maximum accepted fact size (1 MiB) — prevents allocating huge embeddings.
pub const MAX_FACT_BYTES: usize = 1_048_576;

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
