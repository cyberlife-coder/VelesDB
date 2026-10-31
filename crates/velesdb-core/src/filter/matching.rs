//! Condition matching logic and helper functions.

use std::borrow::Cow;

use super::{Condition, POINT_ID_FIELD};
use crate::metrics::global_guardrails_metrics;
use serde_json::Value;

/// What a condition reads: a point's payload and, when the caller knows it,
/// its id, which [`POINT_ID_FIELD`] names.
#[derive(Clone, Copy)]
struct Record<'a> {
    id: Option<u64>,
    payload: &'a Value,
}

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
        // Exactly, as `values_equal` compares two integers: the binary
        // search below must not tie 2^53 with 2^53 + 1.
        (Value::Number(a), Value::Number(b)) => crate::velesql::compare_numbers(a, b),
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
fn match_array_condition(
    record: Record<'_>,
    field: &str,
    values: &[Value],
    mode: ArrayMode,
) -> bool {
    get_field(record, field).is_some_and(|v| match v.as_ref() {
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
    record: Record<'_>,
    field: &str,
    lat: f64,
    lng: f64,
    operator: crate::velesql::CompareOp,
    threshold: f64,
) -> bool {
    get_field(record, field).is_some_and(|v| {
        extract_geo_point(&v).is_some_and(|(plat, plng)| {
            let dist = crate::geo_distance::great_circle_distance_m(plat, plng, lat, lng);
            crate::geo_distance::distance_satisfies(dist, operator.into(), threshold)
        })
    })
}

/// Evaluates a geospatial bounding box check.
fn match_geo_bbox(
    record: Record<'_>,
    field: &str,
    lat_min: f64,
    lng_min: f64,
    lat_max: f64,
    lng_max: f64,
) -> bool {
    get_field(record, field).is_some_and(|v| {
        extract_geo_point(&v).is_some_and(|(plat, plng)| {
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
    /// Evaluates the condition against a payload alone: a `VelesQL` `id`
    /// column reads nothing here, no point id being known. Crate tests only:
    /// every other caller supplies the point id ([`Self::matches_point`]).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn matches(&self, payload: &Value) -> bool {
        self.matches_record(Record { id: None, payload })
    }

    /// Evaluates the condition against a point: its payload (`None` reads
    /// as `null`) and its id, which a `VelesQL` `id` column reads.
    #[must_use]
    pub fn matches_point(&self, id: u64, payload: Option<&Value>) -> bool {
        self.matches_record(Record {
            id: Some(id),
            payload: payload.unwrap_or(&Value::Null),
        })
    }

    fn matches_record(&self, record: Record<'_>) -> bool {
        match self {
            Self::And { conditions } => conditions.iter().all(|c| c.matches_record(record)),
            Self::Or { conditions } => conditions.iter().any(|c| c.matches_record(record)),
            Self::Not { condition } => !condition.matches_record(record),
            Self::Eq { .. }
            | Self::Neq { .. }
            | Self::Gt { .. }
            | Self::Gte { .. }
            | Self::Lt { .. }
            | Self::Lte { .. }
            | Self::In { .. }
            | Self::IsNull { .. }
            | Self::IsNotNull { .. } => self.matches_scalar(record),
            _ => self.matches_text_array_geo(record),
        }
    }

    /// Comparison, `IN` and null leaves.
    fn matches_scalar(&self, record: Record<'_>) -> bool {
        match self {
            Self::Eq { field, value } => {
                get_field(record, field).is_some_and(|v| values_equal(&v, value))
            }
            Self::Neq { field, value } => {
                get_field(record, field).is_none_or(|v| !values_equal(&v, value))
            }
            Self::Gt { .. } | Self::Gte { .. } | Self::Lt { .. } | Self::Lte { .. } => {
                self.matches_ordering(record)
            }
            Self::In { field, values } => {
                get_field(record, field).is_some_and(|v| in_list_matches(&v, values))
            }
            Self::IsNull { field } => get_field(record, field).is_none_or(|v| v.is_null()),
            Self::IsNotNull { field } => get_field(record, field).is_some_and(|v| !v.is_null()),
            _ => false,
        }
    }

    /// `>`, `>=`, `<` and `<=` leaves.
    fn matches_ordering(&self, record: Record<'_>) -> bool {
        use std::cmp::Ordering;
        let (field, value, holds): (&str, &Value, fn(Ordering) -> bool) = match self {
            Self::Gt { field, value } => (field, value, Ordering::is_gt),
            Self::Gte { field, value } => (field, value, Ordering::is_ge),
            Self::Lt { field, value } => (field, value, Ordering::is_lt),
            Self::Lte { field, value } => (field, value, Ordering::is_le),
            _ => return false,
        };
        get_field(record, field).is_some_and(|v| compare_values(&v, value).is_some_and(holds))
    }

    /// Text, array and geospatial leaves.
    fn matches_text_array_geo(&self, record: Record<'_>) -> bool {
        match self {
            Self::Contains { field, value } => get_field(record, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| s.contains(value.as_str()))),
            Self::Like { field, pattern } => get_field(record, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, false))),
            Self::ILike { field, pattern } => get_field(record, field)
                .is_some_and(|v| v.as_str().is_some_and(|s| like_match(s, pattern, true))),
            Self::ArrayContains { .. }
            | Self::ArrayContainsAny { .. }
            | Self::ArrayContainsAll { .. } => self.matches_array(record),
            _ => self.matches_geo(record),
        }
    }

    /// Array leaves.
    fn matches_array(&self, record: Record<'_>) -> bool {
        match self {
            Self::ArrayContains { field, value } => match_array_condition(
                record,
                field,
                std::slice::from_ref(value),
                ArrayMode::Single,
            ),
            Self::ArrayContainsAny { field, values } => {
                match_array_condition(record, field, values, ArrayMode::Any)
            }
            Self::ArrayContainsAll { field, values } => {
                match_array_condition(record, field, values, ArrayMode::All)
            }
            _ => false,
        }
    }

    /// Geospatial leaves.
    fn matches_geo(&self, record: Record<'_>) -> bool {
        match self {
            Self::GeoDistance {
                field,
                lat,
                lng,
                operator,
                threshold,
            } => match_geo_distance(record, field, *lat, *lng, *operator, *threshold),
            Self::GeoBbox {
                field,
                lat_min,
                lng_min,
                lat_max,
                lng_max,
            } => match_geo_bbox(record, field, *lat_min, *lng_min, *lat_max, *lng_max),
            _ => false,
        }
    }
}

/// Gets a field of a record: the point id for [`POINT_ID_FIELD`], else a
/// payload field, supporting dot notation for nested fields.
fn get_field<'a>(record: Record<'a>, field: &str) -> Option<Cow<'a, Value>> {
    if field == POINT_ID_FIELD {
        return record.id.map(|id| Cow::Owned(Value::from(id)));
    }
    let mut current = record.payload;
    for part in field.split('.') {
        current = current.get(part)?;
    }
    Some(Cow::Borrowed(current))
}

/// Compares two JSON values for equality; numbers as [`numbers_equal`].
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => numbers_equal(a, b),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => a == b,
        (Value::Object(a), Value::Object(b)) => a == b,
        _ => false,
    }
}

/// Two floats are equal within `f64::EPSILON`; any other two numbers
/// exactly, as [`compare_values`] orders them (an integer beyond 2^53, as
/// point ids can be, is not the float it rounds to).
fn numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_f64() && b.is_f64() => (x - y).abs() < f64::EPSILON,
        _ => crate::velesql::compare_numbers(a, b).is_eq(),
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
        // Exactly, an integer and a float included (`2^53 + 1 > 2^53.0`).
        (Value::Number(a), Value::Number(b)) => Some(crate::velesql::compare_numbers(a, b)),
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

#[cfg(test)]
#[path = "matching_tests.rs"]
mod tests;
