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
