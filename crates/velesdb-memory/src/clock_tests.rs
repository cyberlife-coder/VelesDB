//! Unit tests for [`super::civil_from_days`] / [`super::today_ymd`] — pinned
//! against known epoch-day references so the hand-rolled calendar conversion
//! (no `chrono` dependency in this crate) is verified independently of
//! whatever day the test happens to run on.

use super::{civil_from_days, today_ymd};

#[test]
fn epoch_day_zero_is_1970_01_01() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
}

#[test]
fn day_18262_is_2020_01_01() {
    // 1577836800 (2020-01-01T00:00:00Z) / 86400 = 18262.
    assert_eq!(civil_from_days(18_262), (2020, 1, 1));
}

#[test]
fn day_18321_is_the_2020_leap_day() {
    // 18262 (2020-01-01) + 31 (January) + 28 = 18321 (2020-02-29).
    assert_eq!(civil_from_days(18_321), (2020, 2, 29));
}

#[test]
fn day_before_the_leap_day_is_still_february() {
    assert_eq!(civil_from_days(18_320), (2020, 2, 28));
}

#[test]
fn today_ymd_is_a_plausible_date() {
    let ymd = today_ymd().expect("native target has a clock");
    assert!(
        (20_240_101..21_000_101).contains(&ymd),
        "today_ymd() returned an implausible date: {ymd}"
    );
}
