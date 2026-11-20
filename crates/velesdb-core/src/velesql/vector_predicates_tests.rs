use super::*;

use serde_json::json;

fn where_of(sql: &str) -> Condition {
    crate::velesql::Parser::parse(sql)
        .expect("test: parse")
        .select
        .where_clause
        .expect("test: a WHERE clause")
}

fn refusal(sql: &str) -> String {
    validate_vector_predicates(&where_of(sql))
        .expect_err("test: refused")
        .to_string()
}

#[test]
fn several_similarity_under_and_filter_in_cascade() {
    let cond =
        where_of("SELECT * FROM d WHERE similarity(vector, $a) > 0.5 AND similarity(e, $b) > 0.3");
    assert!(validate_vector_predicates(&cond).is_ok());
}

#[test]
fn several_similarity_under_one_or_are_refused() {
    let err = refusal(
        "SELECT * FROM d WHERE similarity(vector, $a) > 0.5 OR similarity(vector, $b) > 0.3",
    );
    assert!(err.contains("Multiple similarity()"), "{err}");
}

#[test]
fn a_similarity_or_metadata_union_is_accepted() {
    let cond = where_of("SELECT * FROM d WHERE similarity(vector, $a) > 0.5 OR x = 1");
    assert!(validate_vector_predicates(&cond).is_ok());
}

#[test]
fn near_fused_must_be_the_only_vector_predicate_under_and() {
    for sql in [
        "SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] OR x = 1",
        "SELECT * FROM d WHERE NOT vector NEAR_FUSED [$a, $b]",
        "SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] AND vector NEAR $c",
        "SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] AND similarity(vector, $c) > 0.5",
    ] {
        assert!(refusal(sql).contains("NEAR_FUSED"), "{sql}");
    }
    let cond = where_of("SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] AND x = 1");
    assert!(validate_vector_predicates(&cond).is_ok());
}

#[test]
fn a_named_vector_is_the_payload_numeric_array() {
    let payload = json!({"e": [1, 0.5], "text": "x", "mixed": [1, "a"]});
    assert_eq!(
        payload_vector(&payload, "e").expect("test: ok"),
        Some(vec![1.0, 0.5])
    );
    assert_eq!(payload_vector(&payload, "missing").expect("test: ok"), None);
    assert!(payload_vector(&payload, "text").is_err(), "not an array");
    assert!(
        payload_vector(&payload, "mixed").is_err(),
        "not all numbers"
    );
}

/// A `MATCH` that ranks nothing is a membership test every executor
/// evaluates: under `OR` or `NOT`, or beside a `similarity()` threshold.
#[test]
fn a_match_that_ranks_nothing_is_accepted() {
    for sql in [
        "SELECT * FROM d WHERE body MATCH 'x' OR cat = 'a'",
        "SELECT * FROM d WHERE NOT (body MATCH 'x')",
        "SELECT * FROM d WHERE cat = 'a' AND (body MATCH 'x' OR cat = 'b')",
        "SELECT * FROM d WHERE vector NEAR $v OR body MATCH 'x'",
        "SELECT * FROM d WHERE similarity(vector, $v) > 0.5 AND body MATCH 'x'",
        "SELECT * FROM d WHERE similarity(vector, $v) > 0.5 AND body MATCH 'x' AND title MATCH 'y'",
        "SELECT * FROM d WHERE vector SPARSE_NEAR $s AND NOT (body MATCH 'x')",
        "SELECT * FROM d WHERE vector NEAR $v AND body MATCH 'x' AND NOT (title MATCH 'y')",
    ] {
        assert!(validate_vector_predicates(&where_of(sql)).is_ok(), "{sql}");
    }
}

#[test]
fn a_second_ranking_match_is_refused() {
    for sql in [
        "SELECT * FROM d WHERE body MATCH 'x' AND title MATCH 'y'",
        "SELECT * FROM d WHERE vector NEAR $v AND body MATCH 'x' AND title MATCH 'y'",
    ] {
        let err = refusal(sql);
        assert!(err.contains("2 text MATCH"), "{sql}: {err}");
    }
}

/// No executor fuses a `MATCH` with a sparse branch, a `NEAR_FUSED`, or a
/// `NEAR` beside a `similarity()` threshold.
#[test]
fn a_ranking_match_beside_a_ranking_no_executor_fuses_is_refused() {
    for sql in [
        "SELECT * FROM d WHERE vector SPARSE_NEAR $s AND body MATCH 'x'",
        "SELECT * FROM d WHERE vector NEAR $v AND vector SPARSE_NEAR $s AND body MATCH 'x'",
        "SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] AND body MATCH 'x'",
        "SELECT * FROM d WHERE vector NEAR $v AND similarity(vector, $v) > 0.5 AND body MATCH 'x'",
    ] {
        let err = refusal(sql);
        assert!(err.contains("MATCH"), "{sql}: {err}");
    }
}

#[test]
fn a_match_and_ed_with_metadata_or_one_near_is_accepted() {
    for sql in [
        "SELECT * FROM d WHERE body MATCH 'x'",
        "SELECT * FROM d WHERE body MATCH 'x' AND (cat = 'a' OR NOT (n > 1))",
        "SELECT * FROM d WHERE vector NEAR $v AND body MATCH 'x' AND cat = 'a'",
    ] {
        assert!(validate_vector_predicates(&where_of(sql)).is_ok(), "{sql}");
    }
}

/// A `SPARSE_NEAR` under `NOT` would rank the rows the negation excludes:
/// refused, as a negation the sparse dispatch would drop.
#[test]
fn a_sparse_near_under_not_is_refused() {
    let err = refusal("SELECT * FROM d WHERE NOT (vector SPARSE_NEAR $s) AND cat = 'a'");
    assert!(err.contains("SPARSE_NEAR") && err.contains("NOT"), "{err}");
    assert!(validate_vector_predicates(&where_of(
        "SELECT * FROM d WHERE vector SPARSE_NEAR $s AND NOT (cat = 'a')"
    ))
    .is_ok());
}
