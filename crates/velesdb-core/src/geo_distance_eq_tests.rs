use super::geo_distance_eq::geo_distances_equal;

const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Mirrors `column_store::haversine::haversine_distance` and
/// `filter::matching::haversine_distance_m`: converts each point to radians
/// first, then subtracts.
fn haversine_radians_first(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let (lat1, lng1) = (lat1.to_radians(), lng1.to_radians());
    let (lat2, lng2) = (lat2.to_radians(), lng2.to_radians());
    let dlat = lat2 - lat1;
    let dlng = lng2 - lng1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    EARTH_RADIUS_M * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

/// An equally valid, but numerically distinct, way to compute the same
/// real-world distance: the latitude/longitude difference is taken in
/// degrees first, then converted to radians, instead of converting each
/// point separately and subtracting the radians. A client computing a
/// reference distance a different way (or a different Haversine
/// implementation entirely) is exactly this kind of "same query, different
/// float path" — the case the ULP-scaled tolerance this module replaced
/// could not absorb.
fn haversine_degree_diff_first(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    let (lat1r, lat2r) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlng = (lng2 - lng1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1r.cos() * lat2r.cos() * (dlng / 2.0).sin().powi(2);
    EARTH_RADIUS_M * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

/// Representative coordinate pairs: the original Paris-London report, plus
/// a meridian crossing, a near-antipodal pair, and a near-polar pair —
/// distilled from the 50 000-pair random sweep described in the module
/// docs as producing the largest cross-formula divergence.
const REPRESENTATIVE_PAIRS: &[(f64, f64, f64, f64)] = &[
    (48.8566, 2.3522, 51.5074, -0.1278), // Paris -> London
    (0.0, 179.9, 0.0, -179.9),           // antimeridian crossing
    (-56.236, 71.252, 56.425, -109.324), // near-antipodal, largest ULP gap observed
    (89.9, 0.0, -89.9, 180.0),           // pole to pole
    (48.8566, 2.3522, 48.8566, 2.3522),  // identical point
];

#[test]
fn tolerates_two_equally_valid_haversine_formulas() {
    for &(lat1, lng1, lat2, lng2) in REPRESENTATIVE_PAIRS {
        let a = haversine_radians_first(lat1, lng1, lat2, lng2);
        let b = haversine_degree_diff_first(lat1, lng1, lat2, lng2);
        assert!(
            geo_distances_equal(a, b),
            "pair ({lat1}, {lng1}) -> ({lat2}, {lng2}): {a} vs {b} (diff {})",
            (a - b).abs()
        );
    }
}

#[test]
fn representative_pairs_are_not_bit_identical() {
    // Guards against the test above being vacuous: the two formulas must
    // actually disagree for at least one non-degenerate pair, or
    // `geo_distances_equal` is never exercised on real float noise.
    // Bit-pattern inequality, not a numeric-closeness comparison: the point
    // is that these are two *different* `f64` values, however close.
    let disagreements = REPRESENTATIVE_PAIRS
        .iter()
        .filter(|&&(lat1, lng1, lat2, lng2)| {
            haversine_radians_first(lat1, lng1, lat2, lng2).to_bits()
                != haversine_degree_diff_first(lat1, lng1, lat2, lng2).to_bits()
        })
        .count();
    assert!(disagreements > 0, "no pair exercised any float noise");
}

#[test]
fn still_rejects_a_real_one_meter_difference() {
    let dist = haversine_radians_first(48.8566, 2.3522, 51.5074, -0.1278);
    assert!(!geo_distances_equal(dist, dist + 1.0));
}

#[test]
fn tolerance_holds_well_under_and_rejects_well_over_one_millimeter() {
    let dist = 343_556.06;
    // Comfortable margins on both sides of the 1mm boundary, since the
    // addition itself introduces sub-ULP rounding at this magnitude (see
    // `geo_distance_eq.rs`'s module docs) and a test pinned to the exact
    // boundary value would be at the mercy of that rounding direction.
    assert!(geo_distances_equal(dist, dist + 0.0005));
    assert!(!geo_distances_equal(dist, dist + 0.002));
}
