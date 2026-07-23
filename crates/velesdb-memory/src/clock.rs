//! Wall-clock "today" as a `YYYYMMDD` integer — the single place
//! [`crate::service::MemoryService::remember_with_ttl`] reads the system
//! clock to auto-stamp a fact's [`crate::storage::AUTO_DATE_FIELD`] metadata.
//!
//! Deliberately isolated here and never used by the context compiler pipeline
//! (`context.rs`'s `ContextCompiler::compile`), which stays clock-free and
//! deterministic by design — the same split `context/memory_bridge.rs` already
//! draws for its own `now_nanos`/`now_unix_secs` (event recording stamps
//! wall-clock time; the compile pipeline itself never does). `remember` is a
//! **write** path, already non-deterministic in call time (unlike its
//! content-addressed id), so reading the clock here adds no new
//! nondeterminism to anything that must replay identically.

#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

/// Today's date (UTC) as a `YYYYMMDD` integer, or `None` when no fact should
/// be auto-stamped: `wasm32-unknown-unknown` has no `std` clock
/// (`SystemTime::now()` aborts there — mirrors the guard in
/// `context/memory_bridge.rs`), and a clock reporting a time before the Unix
/// epoch is treated the same way, so a fact is never stamped with a
/// nonsensical date.
#[must_use]
pub(crate) fn today_ymd() -> Option<i64> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let epoch_days = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() / 86_400;
        // Days since the epoch is many orders of magnitude below i64::MAX
        // (u64::MAX / 86_400 alone would take ~2.9e11 years to overflow i64).
        #[allow(clippy::cast_possible_wrap)]
        let (year, month, day) = civil_from_days(epoch_days as i64);
        Some(year * 10_000 + month * 100 + day)
    }
}

/// Days-since-Unix-epoch → `(year, month, day)`, proleptic Gregorian
/// calendar. Howard Hinnant's public-domain `civil_from_days` algorithm
/// (<http://howardhinnant.github.io/date_algorithms.html#civil_from_days>),
/// ported here rather than pulled in as a dependency — this crate has no
/// date/time library; `dated_context.rs` already hand-rolls the inverse
/// direction (`year/month/day` → validated `YYYYMMDD`) the same way.
#[cfg(not(target_arch = "wasm32"))]
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1; // [1, 31]
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "clock_tests.rs"]
mod tests;
