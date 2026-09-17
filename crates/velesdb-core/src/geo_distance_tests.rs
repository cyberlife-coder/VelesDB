//! Tests for `geo_distance`: the formula against high-precision reference
//! distances, the coordinate rule and the equality rule.
//!
//! Every `*_REFERENCE_M` constant below was computed with `mpmath` 1.3.0 at
//! 60 significant digits, from the exact binary value of each `f64` degree
//! input, and rounded to the nearest `f64`. Two formulations were evaluated
//! at that precision, the spherical Vincenty form and the `asin` form of
//! Haversine; they agree to better than 1e-45 m on every pair below, so the
//! reference does not depend on the formula chosen to compute it.

use std::f64::consts::PI;

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use super::geo_distance::{
    distance_satisfies, great_circle_distance_m, DistanceOp, DISTANCE_EQUALITY_RESOLUTION_M,
    EARTH_RADIUS_M,
};

/// How closely a computed distance must reproduce its reference: one
/// micrometre, far above rounding error at half the Earth's circumference
/// (one ULP there is about 3.7e-9 m) and far below any distance a query can
/// tell apart.
const REFERENCE_AGREEMENT_M: f64 = 1e-6;

/// Every operator, for the rules that must hold under all of them.
const ALL_OPS: [DistanceOp; 6] = [
    DistanceOp::Eq,
    DistanceOp::NotEq,
    DistanceOp::Gt,
    DistanceOp::Gte,
    DistanceOp::Lt,
    DistanceOp::Lte,
];

type Pair = (f64, f64, f64, f64);

/// A near-antipodal pair where Haversine's `atan2(√a, √(1 − a))` loses its
/// precision: the previous Haversine implementation was 0.10 m off on it.
const NEAR_ANTIPODAL: Pair = (
    36.308_407_363_430_36,
    54.900_479_056_369_05,
    -36.308_406_445_675_69,
    -125.099_520_729_386_2,
);
const NEAR_ANTIPODAL_REFERENCE_M: f64 = 20_015_086.692_180_898;

/// An exactly antipodal pair for which rounding pushed Haversine's `a`
/// above 1, so `√(1 − a)` and the distance were NaN (#2310).
const NAN_UNDER_HAVERSINE: Pair = (2.5, -120.0, -2.5, 60.0);

/// Paris to London.
const PARIS_LONDON: Pair = (48.8566, 2.3522, 51.5074, -0.1278);
const PARIS_LONDON_REFERENCE_M: f64 = 343_556.060_341_041_65;

/// New York to Los Angeles.
const NYC_LA: Pair = (40.7128, -74.0060, 34.0522, -118.2437);
const NYC_LA_REFERENCE_M: f64 = 3_935_746.254_609_723;

/// 0.2 degrees of longitude apart on the equator, across the antimeridian.
const ACROSS_ANTIMERIDIAN: Pair = (0.0, 179.9, 0.0, -179.9);
const ACROSS_ANTIMERIDIAN_REFERENCE_M: f64 = 22_238.985_328_910_483;

/// Half the Earth's circumference: the distance between antipodes.
const HALF_CIRCUMFERENCE_M: f64 = PI * EARTH_RADIUS_M;

fn distance(pair: Pair) -> f64 {
    let (lat1, lng1, lat2, lng2) = pair;
    great_circle_distance_m(lat1, lng1, lat2, lng2).expect("test: valid coordinates")
}

fn assert_reproduces(pair: Pair, reference_m: f64) {
    let computed = distance(pair);
    assert!(
        (computed - reference_m).abs() < REFERENCE_AGREEMENT_M,
        "{pair:?}: computed {computed} m, reference {reference_m} m"
    );
}

#[test]
fn near_antipodal_pair_reproduces_its_reference() {
    assert_reproduces(NEAR_ANTIPODAL, NEAR_ANTIPODAL_REFERENCE_M);
}

#[test]
fn pair_that_haversine_turned_into_nan_is_half_the_circumference() {
    assert_reproduces(NAN_UNDER_HAVERSINE, HALF_CIRCUMFERENCE_M);
}

#[test]
fn exact_antipodes_on_the_equator_are_half_the_circumference() {
    assert_reproduces((0.0, 0.0, 0.0, 180.0), HALF_CIRCUMFERENCE_M);
}

#[test]
fn pole_to_pole_is_half_the_circumference() {
    assert_reproduces((90.0, 0.0, -90.0, 0.0), HALF_CIRCUMFERENCE_M);
}

#[test]
fn longitude_does_not_move_a_pole() {
    assert_reproduces((90.0, 0.0, 90.0, 123.0), 0.0);
}

#[test]
fn coincident_points_are_exactly_zero_apart() {
    let (lat, lng, _, _) = PARIS_LONDON;
    assert_eq!(distance((lat, lng, lat, lng)).to_bits(), 0.0_f64.to_bits());
}

#[test]
fn distance_across_the_antimeridian_takes_the_short_way() {
    assert_reproduces(ACROSS_ANTIMERIDIAN, ACROSS_ANTIMERIDIAN_REFERENCE_M);
}

#[test]
fn city_pairs_reproduce_their_references() {
    assert_reproduces(PARIS_LONDON, PARIS_LONDON_REFERENCE_M);
    assert_reproduces(NYC_LA, NYC_LA_REFERENCE_M);
}

#[test]
fn distance_is_symmetric() {
    let (lat1, lng1, lat2, lng2) = PARIS_LONDON;
    assert_eq!(
        distance((lat1, lng1, lat2, lng2)).to_bits(),
        distance((lat2, lng2, lat1, lng1)).to_bits()
    );
}

#[test]
fn a_coordinate_off_the_globe_has_no_distance() {
    let (lat1, lng1, lat2, lng2) = PARIS_LONDON;
    for (lat, lng) in [
        (f64::NAN, lng1),
        (lat1, f64::NAN),
        (90.5, lng1),
        (lat1, -180.5),
    ] {
        assert_eq!(
            great_circle_distance_m(lat, lng, lat2, lng2),
            None,
            "({lat}, {lng})"
        );
        assert_eq!(
            great_circle_distance_m(lat2, lng2, lat, lng),
            None,
            "({lat}, {lng})"
        );
    }
}

#[test]
fn equality_is_to_the_millimetre() {
    // Literal on purpose: the rule is "to the millimetre", so this test pins
    // the constant rather than reading it back.
    let dist = distance(PARIS_LONDON);
    let within = dist + 0.000_5;
    let beyond = dist + 0.002;
    assert!(distance_satisfies(Some(dist), DistanceOp::Eq, within));
    assert!(!distance_satisfies(Some(dist), DistanceOp::NotEq, within));
    assert!(!distance_satisfies(Some(dist), DistanceOp::Eq, beyond));
    assert!(distance_satisfies(Some(dist), DistanceOp::NotEq, beyond));
}

#[test]
fn ordering_operators_compare_exactly() {
    let dist = distance(PARIS_LONDON);
    let just_above = dist + DISTANCE_EQUALITY_RESOLUTION_M / 2.0;
    assert!(distance_satisfies(Some(dist), DistanceOp::Lt, just_above));
    assert!(!distance_satisfies(Some(dist), DistanceOp::Gte, just_above));
}

#[test]
fn no_distance_matches_no_operator() {
    for op in ALL_OPS {
        assert!(!distance_satisfies(None, op, 0.0), "{op:?}");
    }
}

#[test]
fn nan_threshold_matches_no_operator() {
    let dist = distance(PARIS_LONDON);
    for op in ALL_OPS {
        assert!(!distance_satisfies(Some(dist), op, f64::NAN), "{op:?}");
    }
}

/// An independent formulation used only by the sweep: the chord between the
/// two unit vectors, turned into an angle through whichever of the direct
/// and the opposite chord is shorter, so `asin` never sees an argument near
/// 1 where it is ill conditioned.
fn chord_distance_m(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let unit = |lat: f64, lng: f64| {
        let (sin_lat, cos_lat) = lat.to_radians().sin_cos();
        let (sin_lng, cos_lng) = lng.to_radians().sin_cos();
        [cos_lat * cos_lng, cos_lat * sin_lng, sin_lat]
    };
    let (a, b) = (unit(lat1, lng1), unit(lat2, lng2));
    let norm = |combine: fn(f64, f64) -> f64| {
        a.iter()
            .zip(b)
            .map(|(x, y)| combine(*x, y).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let direct = norm(|x, y| x - y);
    let opposite = norm(|x, y| x + y);
    if direct <= opposite {
        EARTH_RADIUS_M * 2.0 * (direct / 2.0).asin()
    } else {
        EARTH_RADIUS_M * (PI - 2.0 * (opposite / 2.0).asin())
    }
}

/// A point drawn uniformly over the sphere's surface.
fn random_point(rng: &mut StdRng) -> (f64, f64) {
    // `clamp`: `to_degrees` may round `asin(±1)` just past ±90.
    let lat = rng
        .random_range(-1.0_f64..=1.0)
        .asin()
        .to_degrees()
        .clamp(-90.0, 90.0);
    let lng = rng.random_range(-180.0..=180.0);
    (lat, lng)
}

/// A point within `ANTIPODE_BAND_DEG` degrees, in latitude and longitude,
/// of the antipode of `(lat, lng)`.
fn near_antipode_of(rng: &mut StdRng, lat: f64, lng: f64) -> (f64, f64) {
    const ANTIPODE_BAND_DEG: f64 = 1e-3;
    let band = -ANTIPODE_BAND_DEG..=ANTIPODE_BAND_DEG;
    let antipode_lat = (-lat + rng.random_range(band.clone())).clamp(-90.0, 90.0);
    let mut antipode_lng = lng + 180.0 + rng.random_range(band);
    if antipode_lng > 180.0 {
        antipode_lng -= 360.0;
    }
    (antipode_lat, antipode_lng)
}

/// Re-measures the formula against `chord_distance_m` over a seeded sample:
/// `PAIRS_PER_REGION` pairs drawn uniformly over the sphere, then as many
/// pairs whose second point lies near the first one's antipode, where
/// Haversine failed. Prints the worst gap of each region.
#[test]
#[ignore = "measurement: run with --ignored to re-measure the formula"]
fn sweep_agrees_with_the_chord_formulation() {
    const SEED: u64 = 2299;
    const PAIRS_PER_REGION: usize = 1_000_000;
    let mut rng = StdRng::seed_from_u64(SEED);
    for near_antipode in [false, true] {
        let mut worst_gap_m = 0.0_f64;
        let mut worst_pair = (0.0, 0.0, 0.0, 0.0);
        for _ in 0..PAIRS_PER_REGION {
            let (lat1, lng1) = random_point(&mut rng);
            let (lat2, lng2) = if near_antipode {
                near_antipode_of(&mut rng, lat1, lng1)
            } else {
                random_point(&mut rng)
            };
            let pair = (lat1, lng1, lat2, lng2);
            let gap_m = (distance(pair) - chord_distance_m(lat1, lng1, lat2, lng2)).abs();
            if gap_m > worst_gap_m {
                worst_gap_m = gap_m;
                worst_pair = pair;
            }
        }
        println!(
            "near_antipode={near_antipode}: {PAIRS_PER_REGION} pairs (seed {SEED}), \
             worst gap {worst_gap_m:e} m at {worst_pair:?}"
        );
        assert!(
            worst_gap_m < REFERENCE_AGREEMENT_M,
            "worst gap {worst_gap_m} m at {worst_pair:?}"
        );
    }
}
