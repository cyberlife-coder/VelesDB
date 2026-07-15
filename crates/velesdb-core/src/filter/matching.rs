//! Condition matching logic and helper functions.

use super::Condition;
use crate::metrics::global_guardrails_metrics;
use serde_json::Value;

const LIKE_MAX_PATTERN_BYTES: usize = 4096;
const LIKE_MAX_DYN_OPS: usize = 2_000_000;

/// List length above which IN matching switches from O(n) linear scan to
/// O(log n) binary search. Values must be pre-sorted by [`json_value_cmp`]
/// at `Condition::In` construction time (done in `conversion.rs::convert_in`).
const IN_BINARY_SEARCH_THRESHOLD: usize = 16;

/// Assigns a numeric rank to a JSON value type for consistent total ordering.
///
/// Type rank: Null(0) < Bool(1) < Number(2) < String(3) < Array(4) < Object(5).
fn json_type_rank(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

/// Total ordering over `serde_json::Value` used for sorting IN-list values
/// and binary-searching them at match time (issue #512).
///
/// Within each type: Null = Null, Bool by value, Number by `f64::total_cmp`
/// (NaN sorts after +∞, giving a deterministic total order), String
/// lexicographic, Arrays and Objects by type rank only (equal among themselves,
/// which is acceptable since they are not used in IN conditions in practice).
pub(super) fn json_value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = json_type_rank(a).cmp(&json_type_rank(b));
    if rank != Ordering::Equal {
        return rank;
    }
    match (a, b) {
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .zip(b.as_f64())
            .map_or(Ordering::Equal, |(fa, fb)| fa.total_cmp(&fb)),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Returns whether `field_val` appears in `values`.
///
/// Switches strategy based on list length:
/// - `values.len() <= IN_BINARY_SEARCH_THRESHOLD`: O(n) linear scan via
///   `values_equal` (epsilon-tolerant float equality).
/// - `values.len() > IN_BINARY_SEARCH_THRESHOLD`: O(log n) lower-bound via
///   `json_value_cmp` to narrow position, then `values_equal` for the final
///   check — preserving epsilon semantics on both paths and requiring
///   `values` to be pre-sorted (`conversion.rs::convert_in` guarantees this).
fn in_list_matches(field_val: &Value, values: &[Value]) -> bool {
    if values.len() > IN_BINARY_SEARCH_THRESHOLD {
        debug_assert!(
            values
                .windows(2)
                .all(|w| json_value_cmp(&w[0], &w[1]) != std::cmp::Ordering::Greater),
            "Condition::In values must be pre-sorted by json_value_cmp"
        );
        // Lower-bound: first position where probe is NOT less than field_val.
        let pos = values
            .partition_point(|probe| json_value_cmp(probe, field_val) == std::cmp::Ordering::Less);
        // Check the candidate at pos and its predecessor for epsilon-equal floats.
        let check = |i: usize| values.get(i).is_some_and(|v| values_equal(v, field_val));
        check(pos) || pos > 0 && check(pos - 1)
    } else {
        values.iter().any(|val| values_equal(field_val, val))
    }
}

/// Evaluates array-level conditions (single, any, all).
fn match_array_condition(payload: &Value, field: &str, values: &[Value], mode: ArrayMode) -> bool {
    get_field(payload, field).is_some_and(|v| match v {
        Value::Array(arr) => match mode {
            ArrayMode::Single => values
                .first()
                .is_some_and(|val| arr.iter().any(|e| values_equal(e, val))),
            ArrayMode::Any => values
                .iter()
                .any(|val| arr.iter().any(|e| values_equal(e, val))),
            ArrayMode::All => values
                .iter()
                .all(|val| arr.iter().any(|e| values_equal(e, val))),
        },
        _ => false,
    })
}

/// Array condition matching mode.
#[derive(Clone, Copy)]
enum ArrayMode {
    Single,
    Any,
    All,
}

/// Evaluates geospatial conditions (distance, bounding box).
fn match_geo_distance(
    payload: &Value,
    field: &str,
    lat: f64,
    lng: f64,
    operator: crate::velesql::CompareOp,
    threshold: f64,
) -> bool {
    get_field(payload, field).is_some_and(|v| {
        extract_geo_point(v).is_some_and(|(plat, plng)| {
            let dist = haversine_distance_m(plat, plng, lat, lng);
            compare_geo_distance(dist, threshold, operator)
        })
    })
}

/// Evaluates a geospatial bounding box check.
fn match_geo_bbox(
    payload: &Value,
    field: &str,
    lat_min: f64,
    lng_min: f64,
    lat_max: f64,
    lng_max: f64,
) -> bool {
    get_field(payload, field).is_some_and(|v| {
        extract_geo_point(v).is_some_and(|(plat, plng)| {
            plat >= lat_min && plat <= lat_max && plng >= lng_min && plng <= lng_max
        })
    })
}

/// Extracts `(lat, lng)` from a JSON object with `"lat"` and `"lng"` keys.
fn extract_geo_point(v: &Value) -> Option<(f64, f64)> {
    let lat = v.get("lat").and_then(Value::as_f64)?;
    let lng = v.get("lng").and_then(Value::as_f64)?;
    Some((lat, lng))
}

impl Condition {
    /// Evaluates the condition against a payload.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        match self {
            Self::Eq { field, value } => {
                get_field(payload, field).is_some_and(|v| values_equal(v, value))
            }
            Self::Neq { field, value } => {
                get_field(payload, field).is_none_or(|v| !values_equal(v, value))
            }
            Self::Gt { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_gt)),
            Self::Gte { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_ge)),
            Self::Lt { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_lt)),
            Self::Lte { field, value } => get_field(payload, field)
                .is_some_and(|v| compare_values(v, value).is_some_and(std::cmp::Ordering::is_le)),
            Self::In { field, values } => {
                get_field(payload, field).is_some_and(|v| in_list_matches(v, values))
            }
            Self::Contains { field, value } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| s.contains(value.as_str()))),
            Self::IsNull { field } => get_field(payload, field).is_none_or(Value::is_null),
            Self::IsNotNull { field } => get_field(payload, field).is_some_and(|v| !v.is_null()),
            Self::And { conditions } => conditions.iter().all(|c| c.matches(payload)),
            Self::Or { conditions } => conditions.iter().any(|c| c.matches(payload)),
            Self::Not { condition } => !condition.matches(payload),
            Self::Like { field, pattern } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, false))),
            Self::ILike { field, pattern } => get_field(payload, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, true))),
            Self::ArrayContains { field, value } => match_array_condition(
                payload,
                field,
                std::slice::from_ref(value),
                ArrayMode::Single,
            ),
            Self::ArrayContainsAny { field, values } => {
                match_array_condition(payload, field, values, ArrayMode::Any)
            }
            Self::ArrayContainsAll { field, values } => {
                match_array_condition(payload, field, values, ArrayMode::All)
            }
            Self::GeoDistance {
                field,
                lat,
                lng,
                operator,
                threshold,
            } => match_geo_distance(payload, field, *lat, *lng, *operator, *threshold),
            Self::GeoBbox {
                field,
                lat_min,
                lng_min,
                lat_max,
                lng_max,
            } => match_geo_bbox(payload, field, *lat_min, *lng_min, *lat_max, *lng_max),
        }
    }
}

/// Gets a field from a JSON payload, supporting dot notation for nested fields.
fn get_field<'a>(payload: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = payload;
    for part in field.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

/// Compares two JSON values for equality.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => {
            // Compare as f64 for numeric comparison
            a.as_f64()
                .zip(b.as_f64())
                .is_some_and(|(a, b)| (a - b).abs() < f64::EPSILON)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => a == b,
        (Value::Object(a), Value::Object(b)) => a == b,
        _ => false,
    }
}

/// Compares two JSON values.
///
/// Returns `None` if the types are incompatible (e.g. Number vs String,
/// any type vs Null) or if either number is NaN. Callers must treat `None`
/// as "not comparable" and return `false` for all ordering predicates — this
/// matches SQL three-valued logic where `NULL op X` yields `UNKNOWN` (false).
fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .zip(b.as_f64())
            .and_then(|(fa, fb)| fa.partial_cmp(&fb)),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// SQL LIKE pattern matching implementation.
///
/// Supports:
/// - `%` matches zero or more characters
/// - `_` matches exactly one character
/// - `\%` matches a literal `%`
/// - `\_` matches a literal `_`
///
/// # Performance
///
/// `matches()` is invoked **per candidate row** during a full-scan post-filter,
/// with the *same* `pattern` for every candidate. Naively this re-tokenizes the
/// pattern, re-lowercases it (ILIKE), and allocates two DP buffers on every row.
/// To avoid that, the compiled form (tokens + lowercased pattern for ILIKE) plus
/// the DP scratch buffers are cached in a thread-local [`CompiledLikePattern`]
/// and reused across candidates. Identity is keyed on the *content* of the
/// pattern (and the case-sensitivity flag), so it is correct even if a later
/// query's pattern `String` happens to reuse a freed allocation — no per-row
/// work beyond a length-short-circuited byte comparison of the pattern.
///
/// # Arguments
///
/// * `text` - The string to match against
/// * `pattern` - The SQL LIKE pattern
/// * `case_insensitive` - If true, performs case-insensitive matching (ILIKE)
fn like_match(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    if pattern.len() > LIKE_MAX_PATTERN_BYTES {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return false;
    }

    if text.len().saturating_mul(pattern.len().max(1)) > LIKE_MAX_DYN_OPS {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return false;
    }

    LIKE_COMPILE_CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        // Recompile only when the pattern content or case-sensitivity changed;
        // on a full-scan the same pattern is reused for every candidate row.
        let stale = slot
            .as_ref()
            .is_none_or(|c| !c.matches_key(pattern, case_insensitive));
        if stale {
            *slot = Some(CompiledLikePattern::compile(pattern, case_insensitive));
        }
        match slot.as_mut() {
            Some(compiled) => compiled.run(text),
            // Unreachable in practice (populated above when stale); recompile as
            // a fail-safe rather than unwrap, keeping production code panic-free.
            None => CompiledLikePattern::compile(pattern, case_insensitive).run(text),
        }
    })
}

#[derive(Clone, Copy)]
enum Token {
    AnySeq,
    AnyOne,
    Literal(u8),
}

fn tokenize_like_pattern(pattern: &[u8]) -> Vec<Token> {
    let mut out = Vec::with_capacity(pattern.len());
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' if i + 1 < pattern.len() => {
                out.push(Token::Literal(pattern[i + 1]));
                i += 2;
            }
            b'%' => {
                if !matches!(out.last(), Some(Token::AnySeq)) {
                    out.push(Token::AnySeq);
                }
                i += 1;
            }
            b'_' => {
                out.push(Token::AnyOne);
                i += 1;
            }
            c => {
                out.push(Token::Literal(c));
                i += 1;
            }
        }
    }
    out
}

thread_local! {
    /// Per-thread scratch: the most recently compiled LIKE/ILIKE pattern with
    /// its reusable DP buffers. Bounded to a single pattern; parallel search
    /// threads each keep their own, so there is no contention.
    static LIKE_COMPILE_CACHE: std::cell::RefCell<Option<CompiledLikePattern>> =
        const { std::cell::RefCell::new(None) };
}

/// A LIKE pattern compiled once and reused across candidate rows.
///
/// Holds the tokenized pattern (already lowercased for ILIKE) plus two DP
/// buffers that are resized/reset in place on each [`Self::run`] call rather
/// than reallocated. `source` keeps the *original* pattern bytes so the cache
/// can verify identity by content.
struct CompiledLikePattern {
    /// Original (pre-lowercasing) pattern bytes — the cache identity key.
    source: Vec<u8>,
    case_insensitive: bool,
    tokens: Vec<Token>,
    dp_prev: Vec<bool>,
    dp_curr: Vec<bool>,
}

impl CompiledLikePattern {
    /// Tokenizes `pattern`, lowercasing it first for the case-insensitive path
    /// so the (identical-per-row) pattern lowercase happens exactly once.
    fn compile(pattern: &str, case_insensitive: bool) -> Self {
        let lowered;
        let pattern_bytes = if case_insensitive {
            lowered = pattern.to_lowercase();
            lowered.as_bytes()
        } else {
            pattern.as_bytes()
        };
        Self {
            source: pattern.as_bytes().to_vec(),
            case_insensitive,
            tokens: tokenize_like_pattern(pattern_bytes),
            dp_prev: Vec::new(),
            dp_curr: Vec::new(),
        }
    }

    /// Whether this compiled form is still valid for `(pattern, case)`.
    /// Byte comparison short-circuits on length, so it is cheap.
    fn matches_key(&self, pattern: &str, case_insensitive: bool) -> bool {
        self.case_insensitive == case_insensitive && self.source == pattern.as_bytes()
    }

    /// Runs the rolling-DP match of `text` against the compiled tokens,
    /// reusing the DP buffers (`O(text_len * token_len)` time, `O(token_len)`
    /// memory). For ILIKE the text is lowercased per row (unavoidable, as it
    /// differs across candidates); the pattern lowercase is already amortized.
    fn run(&mut self, text: &str) -> bool {
        let lowered;
        let text_bytes = if self.case_insensitive {
            lowered = text.to_lowercase();
            lowered.as_bytes()
        } else {
            text.as_bytes()
        };

        // Disjoint field borrows so tokens can be read while the DP buffers are
        // mutated.
        let Self {
            tokens,
            dp_prev,
            dp_curr,
            ..
        } = self;
        let n = tokens.len();

        dp_prev.clear();
        dp_prev.resize(n + 1, false);
        dp_prev[0] = true;
        for (j, tok) in tokens.iter().enumerate() {
            if matches!(tok, Token::AnySeq) {
                dp_prev[j + 1] = dp_prev[j];
            } else {
                break;
            }
        }

        dp_curr.clear();
        dp_curr.resize(n + 1, false);
        for &ch in text_bytes {
            dp_curr.fill(false);
            for (j, tok) in tokens.iter().enumerate() {
                dp_curr[j + 1] = match tok {
                    Token::AnySeq => dp_curr[j] || dp_prev[j + 1],
                    Token::AnyOne => dp_prev[j],
                    Token::Literal(c) => dp_prev[j] && ch == *c,
                };
            }
            std::mem::swap(dp_prev, dp_curr);
        }

        dp_prev[n]
    }
}

/// Haversine great-circle distance. Returns distance in **meters** (WGS-84 mean radius).
///
/// Kept local so this module compiles without the `persistence` feature
/// (which gates `column_store::haversine`).
fn haversine_distance_m(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (lat1, lng1) = (lat1.to_radians(), lng1.to_radians());
    let (lat2, lng2) = (lat2.to_radians(), lng2.to_radians());
    let dlat = lat2 - lat1;
    let dlng = lng2 - lng1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    EARTH_RADIUS_M * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

/// Applies a comparison operator to a geo-distance value and threshold.
fn compare_geo_distance(dist: f64, threshold: f64, op: crate::velesql::CompareOp) -> bool {
    use crate::velesql::CompareOp;
    match op {
        CompareOp::Eq => (dist - threshold).abs() < f64::EPSILON,
        CompareOp::NotEq => (dist - threshold).abs() >= f64::EPSILON,
        CompareOp::Gt => dist > threshold,
        CompareOp::Gte => dist >= threshold,
        CompareOp::Lt => dist < threshold,
        CompareOp::Lte => dist <= threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::{like_match, CompiledLikePattern};
    use crate::filter::Condition;
    use serde_json::json;

    fn payload(json: serde_json::Value) -> serde_json::Value {
        json
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
            "hello", "Hello", "HELLO", "hallo", "h_llo", "foobar", "xxfooxx", "abc", "aXbYc",
            "50%", "50x", "a_b", "aXb", "paris", "PARIS", "Paris", "café", "CAFÉ", "", "bc", "zbc",
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
}
