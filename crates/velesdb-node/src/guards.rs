//! DoS caps, validated synchronously on the JS thread BEFORE any work is
//! scheduled on the libuv pool (so an oversized input is rejected immediately
//! rather than after a thread-pool slot is committed).
//!
//! The canonical values live in `velesdb_memory` so all language bindings and
//! the MCP server share a single source of truth without manual syncing.

use crate::error::invalid_input;

pub use velesdb_memory::{MAX_FACT_BYTES, MAX_RECALL_LIMIT, MAX_WHY_HOPS};

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
