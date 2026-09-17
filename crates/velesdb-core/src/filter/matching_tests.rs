use super::{like_match, CompiledLikePattern};
use crate::filter::Condition;
use crate::geo_distance::{great_circle_distance_m, DISTANCE_EQUALITY_RESOLUTION_M};
use crate::velesql::CompareOp;
use serde_json::json;

fn payload(json: serde_json::Value) -> serde_json::Value {
    json
}

const PARIS: (f64, f64) = (48.8566, 2.3522);
const LONDON: (f64, f64) = (51.5074, -0.1278);

/// A payload whose `location` is `(lat, lng)`.
fn located_at((lat, lng): (f64, f64)) -> serde_json::Value {
    payload(json!({"location": {"lat": lat, "lng": lng}}))
}

/// `GEO_DISTANCE(location, lat, lng) operator threshold`.
fn geo_distance((lat, lng): (f64, f64), operator: CompareOp, threshold: f64) -> Condition {
    Condition::GeoDistance {
        field: "location".into(),
        lat,
        lng,
        operator,
        threshold,
    }
}

#[test]
fn geo_distance_eq_is_to_the_millimetre() {
    let london = located_at(LONDON);
    let dist = great_circle_distance_m(LONDON.0, LONDON.1, PARIS.0, PARIS.1)
        .expect("test: valid coordinates");
    let within = dist + DISTANCE_EQUALITY_RESOLUTION_M / 2.0;
    let beyond = dist + DISTANCE_EQUALITY_RESOLUTION_M * 2.0;

    assert!(geo_distance(PARIS, CompareOp::Eq, within).matches(&london));
    assert!(!geo_distance(PARIS, CompareOp::NotEq, within).matches(&london));
    assert!(!geo_distance(PARIS, CompareOp::Eq, beyond).matches(&london));
    assert!(geo_distance(PARIS, CompareOp::NotEq, beyond).matches(&london));
}

// #2310: Haversine gave NaN for this exactly antipodal pair, so the payload
// fell out of every comparison.
#[test]
fn geo_distance_near_the_antipode_is_a_real_distance() {
    let point = located_at((2.5, -120.0));
    assert!(geo_distance((-2.5, 60.0), CompareOp::Gt, 20_000_000.0).matches(&point));
}

// A payload point or a reference point off the globe has no distance, and a
// NaN threshold compares with nothing: `!=` matches no row, like a missing
// location.
#[test]
fn geo_distance_off_globe_point_or_nan_threshold_matches_no_row() {
    let off_globe = located_at((90.5, PARIS.1));
    assert!(!geo_distance(LONDON, CompareOp::NotEq, 0.0).matches(&off_globe));

    let paris = located_at(PARIS);
    assert!(!geo_distance((f64::NAN, LONDON.1), CompareOp::NotEq, 0.0).matches(&paris));
    assert!(!geo_distance(LONDON, CompareOp::NotEq, f64::NAN).matches(&paris));
}

// Verify that comparing against null never matches ordering predicates.
// Previously compare_values returned 0 for incompatible types, causing
// `null >= N` and `null <= N` to spuriously return true (SQL UNKNOWN ≠ true).
#[test]
fn null_field_never_matches_ordering_predicates() {
    let p = payload(json!({"price": null}));
    let n = json!(100);

    assert!(!Condition::Gt {
        field: "price".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Gte {
        field: "price".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Lt {
        field: "price".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Lte {
        field: "price".into(),
        value: n.clone()
    }
    .matches(&p));
}

// A boolean field must not match numeric ordering predicates.
#[test]
fn bool_field_never_matches_numeric_ordering() {
    let p = payload(json!({"active": true}));
    let n = json!(1);

    assert!(!Condition::Gte {
        field: "active".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Lte {
        field: "active".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Gt {
        field: "active".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Lt {
        field: "active".into(),
        value: n.clone()
    }
    .matches(&p));
}

// A string field must not match a numeric comparison operand.
#[test]
fn string_field_never_matches_number_operand() {
    let p = payload(json!({"name": "alice"}));
    let n = json!(100);

    assert!(!Condition::Gt {
        field: "name".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Gte {
        field: "name".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Lt {
        field: "name".into(),
        value: n.clone()
    }
    .matches(&p));
    assert!(!Condition::Lte {
        field: "name".into(),
        value: n.clone()
    }
    .matches(&p));
}

// Sanity: numeric ordering still works for same-type comparisons.
#[test]
fn numeric_ordering_same_type() {
    let p = payload(json!({"price": 50}));

    assert!(Condition::Gt {
        field: "price".into(),
        value: json!(10)
    }
    .matches(&p));
    assert!(Condition::Gte {
        field: "price".into(),
        value: json!(50)
    }
    .matches(&p));
    assert!(!Condition::Gt {
        field: "price".into(),
        value: json!(50)
    }
    .matches(&p));
    assert!(Condition::Lt {
        field: "price".into(),
        value: json!(100)
    }
    .matches(&p));
    assert!(Condition::Lte {
        field: "price".into(),
        value: json!(50)
    }
    .matches(&p));
    assert!(!Condition::Lt {
        field: "price".into(),
        value: json!(50)
    }
    .matches(&p));
}

// ---- LIKE precompiled-pattern equivalence (PERF7) ----

/// Independent oracle: greedy backtracking wildcard matcher, deliberately a
/// *different* algorithm from the production rolling-DP so it cross-checks
/// results rather than re-deriving them the same way.
#[derive(Clone, Copy, PartialEq)]
enum RTok {
    AnySeq,
    AnyOne,
    Lit(u8),
}

fn ref_tokenize(pattern: &[u8]) -> Vec<RTok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' if i + 1 < pattern.len() => {
                out.push(RTok::Lit(pattern[i + 1]));
                i += 2;
            }
            b'%' => {
                out.push(RTok::AnySeq);
                i += 1;
            }
            b'_' => {
                out.push(RTok::AnyOne);
                i += 1;
            }
            c => {
                out.push(RTok::Lit(c));
                i += 1;
            }
        }
    }
    out
}

fn ref_like(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    let (t, p) = if case_insensitive {
        (text.to_lowercase(), pattern.to_lowercase())
    } else {
        (text.to_string(), pattern.to_string())
    };
    let text = t.as_bytes();
    let toks = ref_tokenize(p.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star_j, mut star_i): (Option<usize>, usize) = (None, 0);
    while i < text.len() {
        if j < toks.len() && (toks[j] == RTok::AnyOne || toks[j] == RTok::Lit(text[i])) {
            i += 1;
            j += 1;
        } else if j < toks.len() && toks[j] == RTok::AnySeq {
            star_j = Some(j);
            star_i = i;
            j += 1;
        } else if let Some(sj) = star_j {
            j = sj + 1;
            star_i += 1;
            i = star_i;
        } else {
            return false;
        }
    }
    while j < toks.len() && toks[j] == RTok::AnySeq {
        j += 1;
    }
    j == toks.len()
}

#[test]
fn like_precompiled_matches_reference_over_batch() {
    // Each entry: (pattern, case_insensitive). Interleaved to force cache
    // recompiles and re-hits within one run.
    let patterns: &[(&str, bool)] = &[
        ("hello", false),
        ("%foo%", false),
        ("h_llo", false),
        ("%", false),
        ("", false),
        ("a%b%c", false),
        ("50\\%", false), // escaped percent → literal '%'
        ("a\\_b", false), // escaped underscore → literal '_'
        ("%ARIS", true),  // ILIKE wildcard prefix
        ("PaRiS", true),  // ILIKE mixed case, no wildcard
        ("caf%", true),   // ILIKE over non-ASCII text
        ("_bc", false),
        ("%%%%a", false), // collapsed consecutive %
        ("hello", false), // repeat first pattern (pure cache hit)
    ];
    // A batch of candidate texts covering matches, non-matches, casing,
    // literal wildcards, and unicode.
    let candidates = [
        "hello", "Hello", "HELLO", "hallo", "h_llo", "foobar", "xxfooxx", "abc", "aXbYc", "50%",
        "50x", "a_b", "aXb", "paris", "PARIS", "Paris", "café", "CAFÉ", "", "bc", "zbc",
    ];

    for &(pattern, ci) in patterns {
        // Run the SAME pattern across the whole candidate batch: the first
        // call compiles, the rest are cache hits. Every row must equal both
        // the independent oracle and a freshly-compiled pattern (proving the
        // reused compiled form is not stateful across candidates).
        for text in candidates {
            let via_cache = like_match(text, pattern, ci);
            let expected = ref_like(text, pattern, ci);
            assert_eq!(
                via_cache, expected,
                "cache path diverged from reference: pattern={pattern:?} ci={ci} text={text:?}"
            );

            let mut fresh = CompiledLikePattern::compile(pattern, ci);
            assert_eq!(
                fresh.run(text),
                expected,
                "fresh compile diverged: pattern={pattern:?} ci={ci} text={text:?}"
            );
        }
    }
}

#[test]
fn like_cache_switches_case_flag_for_same_pattern() {
    // Same pattern bytes, different case-sensitivity, alternated: the cache
    // key includes the case flag, so results must never bleed across.
    for _ in 0..8 {
        assert!(!like_match("PARIS", "paris", false)); // LIKE: case matters
        assert!(like_match("PARIS", "paris", true)); // ILIKE: case ignored
        assert!(like_match("paris", "paris", false));
        assert!(like_match("PaRiS", "paris", true));
    }
}

// Sanity: string ordering still works.
#[test]
fn string_ordering_same_type() {
    let p = payload(json!({"name": "bob"}));

    assert!(Condition::Gt {
        field: "name".into(),
        value: json!("alice")
    }
    .matches(&p));
    assert!(Condition::Gte {
        field: "name".into(),
        value: json!("bob")
    }
    .matches(&p));
    assert!(Condition::Lt {
        field: "name".into(),
        value: json!("charlie")
    }
    .matches(&p));
    assert!(Condition::Lte {
        field: "name".into(),
        value: json!("bob")
    }
    .matches(&p));
}
