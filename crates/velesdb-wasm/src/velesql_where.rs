//! WHERE clause evaluation over JSON payloads for the WASM VelesQL executor.
//!
//! Evaluates [`velesdb_core::velesql::Condition`] trees against a
//! `serde_json::Value` payload + a point id, returning a boolean match.
//! Vector/graph/fusion conditions are NOT evaluated here — the executor
//! dispatches them before calling this matcher (see `velesql_select.rs`).

use std::cmp::Ordering;

use velesdb_core::velesql::{CompareOp, Condition, Value};

use crate::velesql_value::{json_values_cmp, json_values_equal, resolve_value, Params};

/// Tests whether the given `id` and optional `payload` match the condition.
///
/// `id` is the point id; the special `id` column is matched against it.
/// Other columns are looked up in the `payload` JSON object (if any).
/// Conditions that require vector/graph/fusion semantics are rejected with
/// a descriptive error, matching the WASM scope.
pub(crate) fn matches(
    cond: &Condition,
    id: u64,
    payload: Option<&serde_json::Value>,
    params: &Params,
) -> Result<bool, String> {
    match cond {
        Condition::Comparison(cmp) => eval_comparison(cmp, id, payload, params),
        Condition::In(c) => eval_in(c, id, payload, params),
        Condition::Between(c) => eval_between(c, id, payload, params),
        Condition::Like(c) => eval_like(c, payload),
        Condition::IsNull(c) => Ok(eval_is_null(c, id, payload)),
        Condition::And(l, r) => Ok(matches(l, id, payload, params)?
            && matches(r, id, payload, params)?),
        Condition::Or(l, r) => Ok(matches(l, id, payload, params)?
            || matches(r, id, payload, params)?),
        Condition::Not(inner) => Ok(!matches(inner, id, payload, params)?),
        Condition::Group(inner) => matches(inner, id, payload, params),
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch(_)
        | Condition::SparseVectorSearch(_) => Err(
            "Vector NEAR clauses are handled by the SELECT dispatcher, not by the WHERE filter"
                .to_string(),
        ),
        Condition::Similarity(_) => Err(
            "similarity() threshold filters are not supported in WASM".to_string(),
        ),
        Condition::GraphMatch(_) => {
            Err("Graph MATCH predicates are not supported in WASM".to_string())
        }
        Condition::Match(_) => {
            Err("MATCH (BM25) conditions are not supported in WASM".to_string())
        }
        Condition::Contains(_) | Condition::ContainsText(_) => {
            Err("CONTAINS / CONTAINS_TEXT conditions are not supported in WASM".to_string())
        }
        Condition::GeoDistance(_) | Condition::GeoBbox(_) => {
            Err("Geospatial conditions are not supported in WASM".to_string())
        }
        // Defensive catch-all: `Condition` is `#[non_exhaustive]`; any new
        // variant added upstream is rejected until explicitly mapped here.
        _ => Err(format!(
            "Unsupported condition variant in WASM WHERE clause: {cond:?}"
        )),
    }
}

/// Looks up a column value:
/// - `id` refers to the point id.
/// - anything else is a payload field (supports dot-nested access).
fn column_value<'a>(
    column: &str,
    id: u64,
    payload: Option<&'a serde_json::Value>,
) -> Option<serde_json::Value> {
    if column == "id" {
        return Some(serde_json::json!(id));
    }
    let payload = payload?;
    crate::filter::get_nested_field(payload, column).cloned()
}

/// Evaluates a comparison (column op value).
fn eval_comparison(
    cmp: &velesdb_core::velesql::Comparison,
    id: u64,
    payload: Option<&serde_json::Value>,
    params: &Params,
) -> Result<bool, String> {
    let right = resolve_value(&cmp.value, params)?;
    let Some(left) = column_value(&cmp.column, id, payload) else {
        // Missing column: only `IS NULL` matches missing, all comparisons are false.
        return Ok(false);
    };
    Ok(match cmp.operator {
        CompareOp::Eq => json_values_equal(&left, &right),
        CompareOp::NotEq => !json_values_equal(&left, &right),
        CompareOp::Gt => json_values_cmp(&left, &right) == Some(Ordering::Greater),
        CompareOp::Gte => matches!(
            json_values_cmp(&left, &right),
            Some(Ordering::Greater | Ordering::Equal)
        ),
        CompareOp::Lt => json_values_cmp(&left, &right) == Some(Ordering::Less),
        CompareOp::Lte => matches!(
            json_values_cmp(&left, &right),
            Some(Ordering::Less | Ordering::Equal)
        ),
        // Reason: `CompareOp` is `#[non_exhaustive]`; new operators default to `false`
        // rather than panicking, until explicitly mapped here.
        _ => false,
    })
}

/// Evaluates an IN / NOT IN condition.
fn eval_in(
    c: &velesdb_core::velesql::InCondition,
    id: u64,
    payload: Option<&serde_json::Value>,
    params: &Params,
) -> Result<bool, String> {
    let Some(left) = column_value(&c.column, id, payload) else {
        return Ok(c.negated); // NOT IN a missing column is true; IN is false.
    };
    let mut found = false;
    for v in &c.values {
        let rv = resolve_value(v, params)?;
        if json_values_equal(&left, &rv) {
            found = true;
            break;
        }
    }
    Ok(if c.negated { !found } else { found })
}

/// Evaluates `column BETWEEN low AND high` (inclusive).
fn eval_between(
    c: &velesdb_core::velesql::BetweenCondition,
    id: u64,
    payload: Option<&serde_json::Value>,
    params: &Params,
) -> Result<bool, String> {
    let Some(left) = column_value(&c.column, id, payload) else {
        return Ok(false);
    };
    let lo = resolve_value(&c.low, params)?;
    let hi = resolve_value(&c.high, params)?;
    let lo_ok = matches!(
        json_values_cmp(&left, &lo),
        Some(Ordering::Greater | Ordering::Equal)
    );
    let hi_ok = matches!(
        json_values_cmp(&left, &hi),
        Some(Ordering::Less | Ordering::Equal)
    );
    Ok(lo_ok && hi_ok)
}

/// Evaluates `column LIKE pattern` with `%` and `_` wildcards.
fn eval_like(
    c: &velesdb_core::velesql::LikeCondition,
    payload: Option<&serde_json::Value>,
) -> Result<bool, String> {
    let Some(payload) = payload else {
        return Ok(false);
    };
    let Some(field_value) = crate::filter::get_nested_field(payload, &c.column) else {
        return Ok(false);
    };
    let Some(text) = field_value.as_str() else {
        return Ok(false);
    };
    Ok(like_match(text, &c.pattern, c.case_insensitive))
}

/// Core LIKE matcher. `%` matches any sequence (including empty), `_` any
/// single character. Case-insensitive when `ci == true`.
fn like_match(text: &str, pattern: &str, ci: bool) -> bool {
    let (text_vec, pat_vec): (Vec<char>, Vec<char>) = if ci {
        (
            text.chars().flat_map(char::to_lowercase).collect(),
            pattern.chars().flat_map(char::to_lowercase).collect(),
        )
    } else {
        (text.chars().collect(), pattern.chars().collect())
    };
    like_rec(&text_vec, 0, &pat_vec, 0)
}

/// Recursive LIKE matcher (handles `%` greedy matching via standard DP-like
/// branching). Input is small (column values), so recursion is safe.
fn like_rec(text: &[char], ti: usize, pat: &[char], pi: usize) -> bool {
    if pi == pat.len() {
        return ti == text.len();
    }
    let p = pat[pi];
    if p == '%' {
        // Skip consecutive % to avoid exponential blowup.
        let mut next_pi = pi + 1;
        while next_pi < pat.len() && pat[next_pi] == '%' {
            next_pi += 1;
        }
        if next_pi == pat.len() {
            return true; // trailing % matches rest.
        }
        for k in ti..=text.len() {
            if like_rec(text, k, pat, next_pi) {
                return true;
            }
        }
        return false;
    }
    if ti == text.len() {
        return false;
    }
    if p == '_' || text[ti] == p {
        return like_rec(text, ti + 1, pat, pi + 1);
    }
    false
}

/// Evaluates `column IS [NOT] NULL`.
fn eval_is_null(
    c: &velesdb_core::velesql::IsNullCondition,
    id: u64,
    payload: Option<&serde_json::Value>,
) -> bool {
    let present = column_value(&c.column, id, payload)
        .is_some_and(|v| !v.is_null());
    if c.is_null {
        !present
    } else {
        present
    }
}

/// Guard used before executing a statement that must not contain vector /
/// graph / fusion logic. Returns an error listing what was rejected.
#[allow(dead_code)]
pub(crate) fn reject_unsupported_where(cond: &Condition) -> Result<(), String> {
    // Walk the condition tree once to catch any unsupported sub-condition.
    match cond {
        Condition::VectorSearch(_)
        | Condition::VectorFusedSearch(_)
        | Condition::SparseVectorSearch(_) => {
            Err("Vector search clauses are not supported here".to_string())
        }
        Condition::GraphMatch(_) => {
            Err("Graph MATCH predicates are not supported in WASM".to_string())
        }
        Condition::And(l, r) | Condition::Or(l, r) => {
            reject_unsupported_where(l)?;
            reject_unsupported_where(r)
        }
        Condition::Not(inner) | Condition::Group(inner) => reject_unsupported_where(inner),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velesql_value::parse_params;
    use velesdb_core::velesql::Parser;

    fn parse_where(sql: &str) -> Condition {
        let q = Parser::parse(sql).expect("test: parse");
        q.select.where_clause.expect("test: has where clause")
    }

    fn empty_params() -> Params {
        parse_params(None).expect("test: empty params")
    }

    #[test]
    fn test_matches_eq_on_id() {
        let c = parse_where("SELECT * FROM t WHERE id = 1");
        assert!(matches(&c, 1, None, &empty_params()).expect("test: eval"));
        assert!(!matches(&c, 2, None, &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_eq_on_payload() {
        let c = parse_where("SELECT * FROM t WHERE cat = 'tech'");
        let payload = serde_json::json!({"cat": "tech"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_gt() {
        let c = parse_where("SELECT * FROM t WHERE price > 10");
        let payload = serde_json::json!({"price": 20});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_gte_and_lte() {
        let c = parse_where("SELECT * FROM t WHERE price >= 5 AND price <= 10");
        let payload = serde_json::json!({"price": 7});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_not_equal() {
        let c = parse_where("SELECT * FROM t WHERE cat != 'tech'");
        let payload = serde_json::json!({"cat": "sport"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_in() {
        let c = parse_where("SELECT * FROM t WHERE cat IN ('tech', 'sport')");
        let payload = serde_json::json!({"cat": "tech"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_not_in() {
        let c = parse_where("SELECT * FROM t WHERE cat NOT IN ('food')");
        let payload = serde_json::json!({"cat": "tech"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_between() {
        let c = parse_where("SELECT * FROM t WHERE price BETWEEN 5 AND 10");
        let payload = serde_json::json!({"price": 7});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_between_boundary_inclusive() {
        let c = parse_where("SELECT * FROM t WHERE price BETWEEN 5 AND 10");
        let low = serde_json::json!({"price": 5});
        let high = serde_json::json!({"price": 10});
        assert!(matches(&c, 0, Some(&low), &empty_params()).expect("test: low"));
        assert!(matches(&c, 0, Some(&high), &empty_params()).expect("test: high"));
    }

    #[test]
    fn test_matches_like_pct_wildcard() {
        let c = parse_where("SELECT * FROM t WHERE name LIKE 'hel%'");
        let payload = serde_json::json!({"name": "hello"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_like_underscore_wildcard() {
        let c = parse_where("SELECT * FROM t WHERE name LIKE 'h_llo'");
        let payload = serde_json::json!({"name": "hello"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_ilike_case_insensitive() {
        let c = parse_where("SELECT * FROM t WHERE name ILIKE 'HEL%'");
        let payload = serde_json::json!({"name": "hello"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_is_null_on_missing_field() {
        let c = parse_where("SELECT * FROM t WHERE title IS NULL");
        let payload = serde_json::json!({"other": "x"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_is_not_null_on_present_field() {
        let c = parse_where("SELECT * FROM t WHERE title IS NOT NULL");
        let payload = serde_json::json!({"title": "x"});
        assert!(matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_matches_and_or_not() {
        let c = parse_where(
            "SELECT * FROM t WHERE (cat = 'tech' OR cat = 'sport') AND NOT (price < 5)",
        );
        let p1 = serde_json::json!({"cat": "tech", "price": 10});
        let p2 = serde_json::json!({"cat": "food", "price": 10});
        let p3 = serde_json::json!({"cat": "tech", "price": 1});
        assert!(matches(&c, 0, Some(&p1), &empty_params()).expect("test: p1"));
        assert!(!matches(&c, 0, Some(&p2), &empty_params()).expect("test: p2"));
        assert!(!matches(&c, 0, Some(&p3), &empty_params()).expect("test: p3"));
    }

    #[test]
    fn test_matches_with_param() {
        let c = parse_where("SELECT * FROM t WHERE price > $threshold");
        let params = parse_params(Some(r#"{"threshold": 10}"#)).expect("test: parse");
        let payload = serde_json::json!({"price": 15});
        assert!(matches(&c, 0, Some(&payload), &params).expect("test: eval"));
    }

    #[test]
    fn test_matches_missing_field_returns_false_for_comparisons() {
        let c = parse_where("SELECT * FROM t WHERE cat = 'tech'");
        let payload = serde_json::json!({"other": "x"});
        assert!(!matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"));
    }

    #[test]
    fn test_like_match_empty_pattern() {
        assert!(like_match("", "", false));
        assert!(!like_match("x", "", false));
    }

    #[test]
    fn test_like_match_only_wildcard() {
        assert!(like_match("anything", "%", false));
        assert!(like_match("", "%", false));
    }

    #[test]
    fn test_like_match_double_wildcard() {
        assert!(like_match("abcdef", "%%", false));
    }
}
