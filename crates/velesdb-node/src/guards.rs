//! DoS caps, validated synchronously on the JS thread BEFORE any work is
//! scheduled on the libuv pool (so an oversized input is rejected immediately
//! rather than after a thread-pool slot is committed).
//!
//! The canonical caps live as private consts in `velesdb-memory`'s `mcp` module,
//! which is gated behind the (off-here) `mcp` feature, so they are re-declared.
//! Keep these in sync with `crates/velesdb-memory/src/mcp.rs`.

use crate::error::invalid_input;

/// Max bytes for a single remembered fact (matches mcp.rs `MAX_FACT_BYTES`).
pub const MAX_FACT_BYTES: usize = 1_048_576;
/// Max results a recall may return; core does not cap `k`, so the adapter does.
pub const MAX_RECALL_LIMIT: usize = 1_000;
/// Max `why()` hop depth (matches mcp.rs / the `PyO3` binding's cap).
pub const MAX_WHY_HOPS: usize = 10;

/// Reject a fact larger than [`MAX_FACT_BYTES`] before scheduling work.
pub fn check_fact(fact: &str) -> napi::Result<()> {
    if fact.len() > MAX_FACT_BYTES {
        return Err(invalid_input(format!(
            "fact exceeds {MAX_FACT_BYTES} bytes ({} given)",
            fact.len()
        )));
    }
    Ok(())
}

/// Clamp a caller-supplied recall limit to [`MAX_RECALL_LIMIT`].
pub fn clamp_limit(k: u32) -> usize {
    (k as usize).min(MAX_RECALL_LIMIT)
}

/// Clamp a caller-supplied `why()` hop budget to [`MAX_WHY_HOPS`].
pub fn clamp_hops(hops: u32) -> usize {
    (hops as usize).min(MAX_WHY_HOPS)
}
