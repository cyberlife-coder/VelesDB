//! `DoS` caps, validated synchronously on the JS thread BEFORE any work is
//! scheduled on the libuv pool (so an oversized input is rejected immediately
//! rather than after a thread-pool slot is committed).
//!
//! The cap *values* and clamping policy are the single source of truth in
//! `velesdb_memory::limits` (no feature gate); this module only adapts them to
//! napi's error channel. No numbers are restated here, so they cannot drift.

use velesdb_memory::limits::{self, MAX_FACT_BYTES};

use crate::error::invalid_input;

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

/// Clamp a caller-supplied recall limit to the shared cap.
pub fn clamp_limit(k: u32) -> usize {
    limits::clamp_recall_limit(k as usize)
}

/// Clamp a caller-supplied `why()` hop budget to the shared cap.
pub fn clamp_hops(hops: u32) -> usize {
    limits::clamp_hops(hops as usize)
}
