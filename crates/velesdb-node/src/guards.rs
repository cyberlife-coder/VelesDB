//! Argument checks and `DoS` caps. Each runs as one of the first lines of a
//! method's task closure — before any embedding or search — so its refusal
//! is a rejected Promise like every other error of this binding.
//!
//! The cap *values* and clamping policy are the single source of truth in
//! `velesdb_memory::limits` (no feature gate); this module only adapts them to
//! napi's error channel. No numbers are restated here, so they cannot drift.

use velesdb_memory::limits::{self, MAX_FACT_BYTES};

use crate::error::invalid_input;

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
/// argument conversion is JS `ToUint32`, which wraps instead of refusing
/// (`Infinity` → 0, `-1` → 4294967295, `2**32 + 60` → 60), so a count
/// arrives as `f64` and is checked here.
pub fn count(name: &str, value: Option<f64>) -> napi::Result<Option<u32>> {
    value.map(|v| whole_u32(name, v)).transpose()
}

fn whole_u32(name: &str, value: f64) -> napi::Result<u32> {
    if value.is_finite() && value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value) {
        // Reason: range-checked just above — a whole number in 0..=u32::MAX.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Ok(value as u32);
    }
    Err(invalid_input(format!(
        "{name} must be a whole number in 0..={}, got {value}",
        u32::MAX
    )))
}

#[cfg(test)]
mod tests {
    //! The crate denies `unwrap`/`expect` in tests too: results are read
    //! through `ok()` / `err()`.
    use super::count;

    #[test]
    fn a_count_is_a_whole_number_in_u32_range() {
        assert_eq!(count("k", None).ok(), Some(None));
        assert_eq!(count("k", Some(0.0)).ok(), Some(Some(0)));
        assert_eq!(
            count("k", Some(f64::from(u32::MAX))).ok(),
            Some(Some(u32::MAX))
        );
        for bad in [
            f64::INFINITY,
            f64::NAN,
            -1.0,
            f64::from(u32::MAX) + 1.0,
            1.5,
        ] {
            let reason = count("k", Some(bad)).err().map(|e| e.reason);
            assert!(
                reason
                    .as_deref()
                    .is_some_and(|r| r.contains("INVALID_INPUT")),
                "{bad}: {reason:?}"
            );
        }
    }
}
