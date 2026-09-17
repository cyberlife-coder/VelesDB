//! Great-circle distance and its comparison, shared by every `GEO_DISTANCE`
//! evaluation path.
//!
//! `column_store::filter_geo` (the public `ColumnStore` filter API) and
//! `filter::matching` (`VelesQL`'s payload evaluation of `GEO_DISTANCE`) both
//! call [`great_circle_distance_m`] and [`distance_satisfies`], so the
//! formula, the coordinate rule and the equality rule cannot drift between
//! them.
//!
//! # Formula
//!
//! The central angle is computed with the spherical special case of
//! Vincenty's formula:
//!
//! ```text
//! c = atan2( sqrt((cos φ2 · sin Δλ)² + (cos φ1 · sin φ2 − sin φ1 · cos φ2 · cos Δλ)²),
//!            sin φ1 · sin φ2 + cos φ1 · cos φ2 · cos Δλ )
//! d = R · c
//! ```
//!
//! It is well conditioned for every pair of points. Haversine's
//! `2 · atan2(√a, √(1 − a))`, used before, is not: near the antipode `a`
//! tends to 1, `1 − a` loses its significant digits, and rounding can push
//! `a` above 1 so that `√(1 − a)` is NaN (#2310). Here no square root ever
//! takes a difference that can go negative, so the result is finite for
//! every valid coordinate pair.
//!
//! `geo_distance_tests` checks the formula against reference distances
//! computed at 60 significant digits, and its ignored
//! `sweep_agrees_with_the_chord_formulation` test compares it against an
//! independent formulation over a seeded sample that covers both the whole
//! sphere and the neighbourhood of the antipode.
//!
//! # Coordinates
//!
//! A latitude outside `[-90, 90]` or a longitude outside `[-180, 180]`
//! (NaN included) names no point on Earth, so it has no distance: the
//! distance is `None`, and [`distance_satisfies`] matches no row for it,
//! under every operator, `!=` included, exactly like a null `GeoPoint`.
//!
//! # Equality
//!
//! `GEO_DISTANCE(...) = X` and `!= X` follow a product rule, not a
//! floating-point noise bound: two distances are equal when they agree to
//! the millimetre ([`DISTANCE_EQUALITY_RESOLUTION_M`]). A NaN threshold
//! matches no row under any operator.

use std::ops::RangeInclusive;

/// Mean Earth radius in meters, rounded to the kilometre (6 371 km).
pub(crate) const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Latitudes, in degrees, that name a point on Earth.
pub(crate) const LATITUDE_RANGE_DEG: RangeInclusive<f64> = -90.0..=90.0;

/// Longitudes, in degrees, that name a point on Earth.
pub(crate) const LONGITUDE_RANGE_DEG: RangeInclusive<f64> = -180.0..=180.0;

/// Two distances are equal under `GEO_DISTANCE(...) = X` when they differ by
/// less than this many meters: they agree to the millimetre.
///
/// A product rule chosen for what a distance query discriminates, not a
/// bound on the formula's rounding error, which is many orders of magnitude
/// smaller (see the module docs for the tests that measure it).
pub(crate) const DISTANCE_EQUALITY_RESOLUTION_M: f64 = 1e-3;

/// A comparison of a computed distance against a threshold.
///
/// The column-store and `VelesQL` comparison enums both convert into it, so
/// the rule lives in one place without the column store depending on the
/// query parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DistanceOp {
    /// `=`: equal to the millimetre.
    Eq,
    /// `!=`: not equal to the millimetre.
    NotEq,
    /// `>`
    Gt,
    /// `>=`
    Gte,
    /// `<`
    Lt,
    /// `<=`
    Lte,
}

/// `From<$source> for DistanceOp`, for a comparison enum with the same six
/// variants; the match stays exhaustive, so a new variant fails to compile.
macro_rules! distance_op_from {
    ($(#[$attr:meta])* $source:path) => {
        $(#[$attr])*
        impl From<$source> for DistanceOp {
            fn from(op: $source) -> Self {
                type Source = $source;
                match op {
                    Source::Eq => Self::Eq,
                    Source::NotEq => Self::NotEq,
                    Source::Gt => Self::Gt,
                    Source::Gte => Self::Gte,
                    Source::Lt => Self::Lt,
                    Source::Lte => Self::Lte,
                }
            }
        }
    };
}

distance_op_from!(crate::velesql::CompareOp);
distance_op_from!(
    #[cfg(feature = "persistence")]
    crate::column_store::CompareOp
);

/// Whether `(lat, lng)`, in degrees, names a point on Earth.
///
/// NaN is outside every range, so it is never a valid coordinate.
#[must_use]
pub(crate) fn is_valid_coordinate(lat: f64, lng: f64) -> bool {
    LATITUDE_RANGE_DEG.contains(&lat) && LONGITUDE_RANGE_DEG.contains(&lng)
}

/// Great-circle distance in meters between two points given in degrees.
///
/// Returns `None` when either point is not a valid coordinate (see
/// [`is_valid_coordinate`]); otherwise the distance is finite and lies in
/// `[0, π · EARTH_RADIUS_M]`. Pure and allocation-free.
#[must_use]
pub(crate) fn great_circle_distance_m(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> Option<f64> {
    if !is_valid_coordinate(lat1, lng1) || !is_valid_coordinate(lat2, lng2) {
        return None;
    }
    // The terms below are not symmetric in rounding; ordering the points
    // makes the distance from A to B bit-identical to the distance from B to A.
    let ((lat1, lng1), (lat2, lng2)) = if (lat1, lng1) <= (lat2, lng2) {
        ((lat1, lng1), (lat2, lng2))
    } else {
        ((lat2, lng2), (lat1, lng1))
    };
    let (sin_phi1, cos_phi1) = lat1.to_radians().sin_cos();
    let (sin_phi2, cos_phi2) = lat2.to_radians().sin_cos();
    let (sin_dlambda, cos_dlambda) = (lng2.to_radians() - lng1.to_radians()).sin_cos();
    let east = cos_phi2 * sin_dlambda;
    let north = cos_phi1 * sin_phi2 - sin_phi1 * cos_phi2 * cos_dlambda;
    let along = sin_phi1 * sin_phi2 + cos_phi1 * cos_phi2 * cos_dlambda;
    let central_angle = (east * east + north * north).sqrt().atan2(along);
    Some(EARTH_RADIUS_M * central_angle)
}

/// Whether a computed distance satisfies `distance op threshold`.
///
/// A `None` distance (an invalid coordinate) or a NaN threshold matches
/// nothing, under every operator. `Eq`/`NotEq` compare to the millimetre
/// ([`DISTANCE_EQUALITY_RESOLUTION_M`]); the ordering operators compare
/// exactly.
#[must_use]
pub(crate) fn distance_satisfies(distance: Option<f64>, op: DistanceOp, threshold: f64) -> bool {
    let Some(distance) = distance else {
        return false;
    };
    if threshold.is_nan() {
        return false;
    }
    let equal = (distance - threshold).abs() < DISTANCE_EQUALITY_RESOLUTION_M;
    match op {
        DistanceOp::Eq => equal,
        DistanceOp::NotEq => !equal,
        DistanceOp::Gt => distance > threshold,
        DistanceOp::Gte => distance >= threshold,
        DistanceOp::Lt => distance < threshold,
        DistanceOp::Lte => distance <= threshold,
    }
}
