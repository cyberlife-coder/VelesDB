//! Condition matching logic and helper functions.

use std::borrow::Cow;

use super::{Condition, InValues, POINT_ID_FIELD};
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
/// O(log n) binary search. [`InValues`] sorts by [`json_value_cmp`] at
/// `Condition::In` construction time, so the sortedness the search needs is
/// a property of the type, not of the caller.
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
/// Within each type: Null = Null, Bool by value, Number by exact value
/// ([`compare_numbers`](crate::velesql::compare_numbers), which a JSON
/// number's finiteness makes total), String
/// lexicographic, Arrays and Objects by type rank only (equal among
/// themselves — [`in_list_matches`] scans them linearly, as no lower bound
/// could pick a member out of the equal run).
pub(super) fn json_value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = json_type_rank(a).cmp(&json_type_rank(b));
    if rank != Ordering::Equal {
        return rank;
    }
    match (a, b) {
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        // The same order `values_equal` calls equal: the binary search below
        // must not tie 2^53 with 2^53 + 1, nor split `-0.0` from `0`.
        (Value::Number(a), Value::Number(b)) => crate::velesql::compare_numbers(a, b),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Returns whether `field_val` appears in `values`.
///
/// Switches strategy on list length and probe type:
/// - `values.len() <= IN_BINARY_SEARCH_THRESHOLD`: O(n) linear scan via
///   `values_equal` (exact value equality).
/// - Longer list, scalar probe (null, bool, number, string): O(log n)
///   lower-bound via `json_value_cmp` to narrow position, then
///   `values_equal` for the final check — within a scalar rank
///   `json_value_cmp` levels exactly the values `values_equal` calls equal,
///   so a hit can only sit at the lower bound. Requires `values` sorted,
///   which [`InValues`] guarantees at construction.
/// - Array or object probe: `json_value_cmp` levels all arrays (all
///   objects) to Equal, so no lower bound picks the member — a linear
///   `values_equal` scan instead.
fn in_list_matches(field_val: &Value, values: &InValues) -> bool {
    let scalar_probe = !matches!(field_val, Value::Array(_) | Value::Object(_));
    if values.len() > IN_BINARY_SEARCH_THRESHOLD && scalar_probe {
        // Lower-bound: first position where probe is NOT less than field_val.
        let pos = values
            .partition_point(|probe| json_value_cmp(probe, field_val) == std::cmp::Ordering::Less);
        values.get(pos).is_some_and(|v| values_equal(v, field_val))
    } else {
        values.iter().any(|val| values_equal(field_val, val))
    }
}

/// Evaluates array-level conditions (single, any, all) on a known operand:
/// an array is tested, any other value is known not to contain anything.
fn match_array_condition(operand: &Value, values: &[Value], mode: ArrayMode) -> bool {
    match operand {
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
    }
}

/// Array condition matching mode.
#[derive(Clone, Copy)]
enum ArrayMode {
    Single,
    Any,
    All,
}

/// Evaluates a geospatial distance on a known operand; a value that is not
/// a `{lat, lng}` point is known not to satisfy it.
fn match_geo_distance(
    operand: &Value,
    lat: f64,
    lng: f64,
    operator: crate::velesql::CompareOp,
    threshold: f64,
) -> bool {
    extract_geo_point(operand).is_some_and(|(plat, plng)| {
        let dist = crate::geo_distance::great_circle_distance_m(plat, plng, lat, lng);
        crate::geo_distance::distance_satisfies(dist, operator.into(), threshold)
    })
}

/// Evaluates a geospatial bounding box on a known operand.
fn match_geo_bbox(operand: &Value, lat_min: f64, lng_min: f64, lat_max: f64, lng_max: f64) -> bool {
    extract_geo_point(operand).is_some_and(|(plat, plng)| {
        plat >= lat_min && plat <= lat_max && plng >= lng_min && plng <= lng_max
    })
}

/// Extracts `(lat, lng)` from a JSON object with `"lat"` and `"lng"` keys.
fn extract_geo_point(v: &Value) -> Option<(f64, f64)> {
    let lat = v.get("lat").and_then(Value::as_f64)?;
    let lng = v.get("lng").and_then(Value::as_f64)?;
    Some((lat, lng))
}

/// Kleene AND: known false as soon as one operand is, else UNKNOWN as soon
/// as one is, else true (`AND` of nothing is true).
fn kleene_all(truths: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut acc = Some(true);
    for truth in truths {
        match truth {
            Some(false) => return Some(false),
            None => acc = None,
            Some(true) => {}
        }
    }
    acc
}

/// Kleene OR: known true as soon as one operand is, else UNKNOWN as soon as
/// one is, else false (`OR` of nothing is false).
fn kleene_any(truths: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut acc = Some(false);
    for truth in truths {
        match truth {
            Some(true) => return Some(true),
            None => acc = None,
            Some(false) => {}
        }
    }
    acc
}

impl Condition {
    /// Evaluates the condition against a payload alone: a `VelesQL` `id`
    /// column reads nothing here, no point id being known. Crate tests only:
    /// every other caller supplies the point id ([`Self::matches_point`]).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn matches(&self, payload: &Value) -> bool {
        self.eval_record(Record { id: None, payload }) == Some(true)
    }

    /// Whether the point is admitted: its condition is known TRUE. Its
    /// payload (`None` reads as `null`) and its id, which a `VelesQL` `id`
    /// column reads, are what the condition tests.
    #[must_use]
    pub fn matches_point(&self, id: u64, payload: Option<&Value>) -> bool {
        self.evaluate_point(id, payload) == Some(true)
    }

    /// Evaluates the condition against a point in SQL's three-valued logic:
    /// `Some(true)`, `Some(false)`, or `None` for UNKNOWN — a comparison
    /// whose operand is `null` or absent. An enclosing `NOT` keeps UNKNOWN
    /// unknown, so a caller combining this with other predicates must not
    /// fold `None` into `false` before it has applied every negation.
    #[must_use]
    pub(crate) fn evaluate_point(&self, id: u64, payload: Option<&Value>) -> Option<bool> {
        self.eval_record(Record {
            id: Some(id),
            payload: payload.unwrap_or(&Value::Null),
        })
    }

    fn eval_record(&self, record: Record<'_>) -> Option<bool> {
        match self {
            Self::And { conditions } => {
                kleene_all(conditions.iter().map(|c| c.eval_record(record)))
            }
            Self::Or { conditions } => kleene_any(conditions.iter().map(|c| c.eval_record(record))),
            Self::Not { condition } => condition.eval_record(record).map(|known| !known),
            Self::IsNull { field } => Some(operand(record, field).is_none()),
            Self::IsNotNull { field } => Some(operand(record, field).is_some()),
            // An empty disjunction is false whatever the field holds: no
            // comparison with it is made, so `null` cannot make it UNKNOWN.
            Self::In { values, .. } if values.is_empty() => Some(false),
            Self::ArrayContainsAny { values, .. } if values.is_empty() => Some(false),
            leaf => {
                let field = leaf.leaf_field()?;
                let value = operand(record, field)?;
                leaf.eval_known(&value)
            }
        }
    }

    /// The field a comparison, text, array or geospatial leaf reads.
    fn leaf_field(&self) -> Option<&str> {
        match self {
            Self::Eq { field, .. }
            | Self::Neq { field, .. }
            | Self::Gt { field, .. }
            | Self::Gte { field, .. }
            | Self::Lt { field, .. }
            | Self::Lte { field, .. }
            | Self::In { field, .. }
            | Self::Contains { field, .. }
            | Self::Like { field, .. }
            | Self::ILike { field, .. }
            | Self::ArrayContains { field, .. }
            | Self::ArrayContainsAny { field, .. }
            | Self::ArrayContainsAll { field, .. }
            | Self::GeoDistance { field, .. }
            | Self::GeoBbox { field, .. } => Some(field),
            _ => None,
        }
    }

    /// Evaluates a leaf on a known (present, non-`null`) operand. A `NULL`
    /// literal still makes a comparison UNKNOWN, as in SQL (`x = NULL`); a
    /// value of another type than the one tested is known not to satisfy it.
    fn eval_known(&self, operand: &Value) -> Option<bool> {
        use std::cmp::Ordering;
        let ordering = |value: &Value, holds: fn(Ordering) -> bool| {
            known_literal(value)?;
            Some(compare_values(operand, value).is_some_and(holds))
        };
        match self {
            Self::Eq { value, .. } => known_literal(value).map(|v| values_equal(operand, v)),
            Self::Neq { value, .. } => known_literal(value).map(|v| !values_equal(operand, v)),
            Self::Gt { value, .. } => ordering(value, Ordering::is_gt),
            Self::Gte { value, .. } => ordering(value, Ordering::is_ge),
            Self::Lt { value, .. } => ordering(value, Ordering::is_lt),
            Self::Lte { value, .. } => ordering(value, Ordering::is_le),
            Self::In { values, .. } => in_list_truth(operand, values),
            Self::Contains { value, .. } => {
                Some(operand.as_str().is_some_and(|s| s.contains(value.as_str())))
            }
            Self::Like { pattern, .. } => like_truth(operand, pattern, false),
            Self::ILike { pattern, .. } => like_truth(operand, pattern, true),
            Self::ArrayContains { value, .. } => Some(match_array_condition(
                operand,
                std::slice::from_ref(value),
                ArrayMode::Single,
            )),
            Self::ArrayContainsAny { values, .. } => {
                Some(match_array_condition(operand, values, ArrayMode::Any))
            }
            Self::ArrayContainsAll { values, .. } => {
                Some(match_array_condition(operand, values, ArrayMode::All))
            }
            Self::GeoDistance {
                lat,
                lng,
                operator,
                threshold,
                ..
            } => Some(match_geo_distance(
                operand, *lat, *lng, *operator, *threshold,
            )),
            Self::GeoBbox {
                lat_min,
                lng_min,
                lat_max,
                lng_max,
                ..
            } => Some(match_geo_bbox(
                operand, *lat_min, *lng_min, *lat_max, *lng_max,
            )),
            _ => None,
        }
    }
}

/// A record's field as a comparison operand: `None` when it is absent or
/// `null` — SQL's `NULL`, which no comparison decides.
fn operand<'a>(record: Record<'a>, field: &str) -> Option<Cow<'a, Value>> {
    get_field(record, field).filter(|v| !v.is_null())
}

/// A comparison literal, or `None` for `NULL`, which makes it UNKNOWN.
fn known_literal(value: &Value) -> Option<&Value> {
    (!value.is_null()).then_some(value)
}

/// `operand IN (values)`: true on a member, else UNKNOWN when the list holds
/// a `NULL` (the operand might be it), else false. [`InValues`] sorts `null`
/// first ([`json_value_cmp`] ranks it lowest), so the check is O(1).
fn in_list_truth(operand: &Value, values: &InValues) -> Option<bool> {
    if in_list_matches(operand, values) {
        Some(true)
    } else if values.first().is_some_and(Value::is_null) {
        None
    } else {
        Some(false)
    }
}

/// LIKE / ILIKE on a known operand: a non-string is known not to match; a
/// pattern the guardrails refuse to run leaves the predicate UNKNOWN, so an
/// enclosing `NOT` cannot admit a row nothing was matched against.
fn like_truth(operand: &Value, pattern: &str, case_insensitive: bool) -> Option<bool> {
    match operand.as_str() {
        Some(text) => like_match(text, pattern, case_insensitive),
        None => Some(false),
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

/// Compares two JSON values for equality; numbers by exact value, as
/// [`crate::velesql::compare_numbers`] orders them (`-0.0` is `0`, an
/// integer beyond 2^53 is not the float it rounds to, and no tolerance
/// makes two different values equal).
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => crate::velesql::compare_numbers(a, b).is_eq(),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => a == b,
        (Value::Object(a), Value::Object(b)) => a == b,
        _ => false,
    }
}

/// Compares two JSON values.
///
/// Returns `None` if the types are incompatible (e.g. Number vs String):
/// such values are known not to satisfy any ordering. `NULL` operands never
/// reach it — [`operand`] and [`known_literal`] make those UNKNOWN first.
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
fn like_match(text: &str, pattern: &str, case_insensitive: bool) -> Option<bool> {
    if pattern.len() > LIKE_MAX_PATTERN_BYTES {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return None;
    }

    if text.len().saturating_mul(pattern.len().max(1)) > LIKE_MAX_DYN_OPS {
        global_guardrails_metrics().record_like_guardrail_rejected();
        return None;
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
        Some(match slot.as_mut() {
            Some(compiled) => compiled.run(text),
            // Unreachable in practice (populated above when stale); recompile as
            // a fail-safe rather than unwrap, keeping production code panic-free.
            None => CompiledLikePattern::compile(pattern, case_insensitive).run(text),
        })
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
