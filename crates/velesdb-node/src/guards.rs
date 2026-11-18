//! Argument checks and `DoS` caps. Each runs as one of the first lines of a
//! method's task closure — before any embedding or search — so its refusal
//! is a rejected Promise like every other error of this binding.
//!
//! The cap *values* and clamping policy are the single source of truth in
//! `velesdb_memory::limits` (no feature gate); this module only adapts them to
//! napi's error channel. No numbers are restated here, so they cannot drift.

use velesdb_memory::limits::{self, MAX_FACT_BYTES};

use crate::error::{invalid_input, to_napi_err};

/// Reject a fact larger than [`MAX_FACT_BYTES`] before any work on it.
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

/// A caller-supplied count — a result limit, a TTL, a hop budget, a token
/// reserve, an index — read as JS gives it: a finite whole number in
/// `0..=u32::MAX`, refused with `INVALID_INPUT` otherwise. napi's own `u32`
/// argument conversion is JS `ToUint32`, which wraps instead of refusing, so
/// a count arrives as `f64` and is checked by the validator every JS binding
/// shares, [`limits::whole_count`]; this only maps its refusal onto napi's
/// error channel.
pub fn count(name: &'static str, value: Option<f64>) -> napi::Result<Option<u32>> {
    limits::whole_count(name, value).map_err(to_napi_err)
}

#[cfg(test)]
#[path = "guards_tests.rs"]
mod guards_tests;
