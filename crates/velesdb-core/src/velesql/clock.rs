//! The wall clock `NOW()` reads.
//!
//! `wasm32-unknown-unknown` has no system clock — `std::time::SystemTime::now`
//! panics there — so in a browser the time comes from the page
//! (`Date.now()`, through `web-time`); every other target reads the system
//! clock.

#[cfg(target_arch = "wasm32")]
use web_time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, `0` for a clock set before it and
/// saturating at `i64::MAX`.
pub(super) fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
