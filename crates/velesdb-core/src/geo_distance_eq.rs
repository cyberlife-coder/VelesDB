//! Equality tolerance for computed geo (Haversine) distance comparisons.
//!
//! `GEO_DISTANCE(...) = X` / `!= X` compares a *computed* great-circle
//! distance against a caller-supplied threshold. Two independently valid
//! ways of computing that same real-world distance — for instance,
//! subtracting latitudes in radians (this crate's own
//! `haversine_distance`/`haversine_distance_m`) versus taking the
//! difference in degrees first and converting to radians afterward — do
//! not produce bit-identical `f64` results, so an equality check tight
//! enough to demand bit-identical inputs is unusable in practice.
//!
//! A tolerance scaled to a handful of ULPs at the compared magnitude (the
//! natural first guess, and the one this module replaces) is not enough:
//! across 50 000 uniformly-random point pairs spanning the full
//! latitude/longitude range, the two formulas above disagreed by up to
//! ~4.4e-7 meters (see `geo_distance_eq_tests` for the fixed reproduction
//! cases distilled from that sweep). [`GEO_DISTANCE_EQ_TOLERANCE_M`] is
//! set several orders of magnitude above that observed ceiling, while
//! staying far below any distance granularity a real query would care
//! about, so it absorbs cross-formula floating-point noise without
//! treating genuinely different distances as equal.
//!
//! Used by both `column_store::filter_geo` (the public `ColumnStore`
//! filter API) and `filter::matching` (`VelesQL`'s own `GEO_DISTANCE`
//! evaluation path) so the two never drift apart again.

/// Two computed geo-distances (in meters) within this of each other are
/// treated as equal by `GEO_DISTANCE(...) = X` / `!= X`.
///
/// One millimeter: about 2 000x the largest cross-formula floating-point
/// divergence observed in the sweep described in the module docs, and far
/// finer than any real-world distance query needs to discriminate.
const GEO_DISTANCE_EQ_TOLERANCE_M: f64 = 1e-3;

/// Returns `true` if two computed geo-distances (in meters) should compare
/// equal under `GEO_DISTANCE(...) = X`.
#[must_use]
pub(crate) fn geo_distances_equal(a: f64, b: f64) -> bool {
    (a - b).abs() < GEO_DISTANCE_EQ_TOLERANCE_M
}
