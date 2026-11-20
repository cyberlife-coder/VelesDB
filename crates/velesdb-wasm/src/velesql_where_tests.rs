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
    let c =
        parse_where("SELECT * FROM t WHERE (cat = 'tech' OR cat = 'sport') AND NOT (price < 5)");
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

/// GIVEN a payload without the field a negative predicate reads
/// WHEN WASM evaluates `!=` or `NOT IN` on it
/// THEN it does not match, as core's filter and the server answer: a missing
///      field makes the predicate UNKNOWN, and WHERE admits only TRUE.
#[test]
fn test_negative_predicates_on_a_missing_field_match_like_core() {
    let payload = serde_json::json!({"other": "x"});
    for sql in [
        "SELECT * FROM t WHERE cat != 'tech'",
        "SELECT * FROM t WHERE cat NOT IN ('tech', 'sport')",
        "SELECT * FROM t WHERE NOT (cat = 'tech')",
    ] {
        let c = parse_where(sql);
        assert!(
            !matches(&c, 0, Some(&payload), &empty_params()).expect("test: eval"),
            "{sql}"
        );
    }
}

/// GIVEN a LIKE pattern escaping a wildcard with a backslash
/// WHEN WASM evaluates it
/// THEN the escaped `%` / `_` matches itself only, as in core.
#[test]
fn test_like_escaped_wildcard_matches_itself_like_core() {
    let c = parse_where(r"SELECT * FROM t WHERE code LIKE '100\%'");
    let literal = serde_json::json!({"code": "100%"});
    let longer = serde_json::json!({"code": "1000"});
    assert!(matches(&c, 0, Some(&literal), &empty_params()).expect("test: eval"));
    assert!(!matches(&c, 0, Some(&longer), &empty_params()).expect("test: eval"));
}

/// GIVEN a `$param` bound to an array, which no scalar comparison takes
/// WHEN WASM evaluates the comparison
/// THEN it refuses the parameter, as core does, instead of comparing it.
#[test]
fn test_array_bound_scalar_param_is_refused_like_core() {
    let c = parse_where("SELECT * FROM t WHERE cat = $v");
    let params = parse_params(Some(r#"{"v": ["tech"]}"#)).expect("test: parse");
    let payload = serde_json::json!({"cat": ["tech"]});
    let err = matches(&c, 0, Some(&payload), &params).expect_err("test: refused");
    assert!(
        err.to_string().contains("[VELES-038]") && err.to_string().contains("$v"),
        "{err}"
    );
}
