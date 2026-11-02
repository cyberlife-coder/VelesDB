//! BDD-style end-to-end tests: numbers compare by exact value. An integer
//! and a float compare by the values they stand for, `-0.0` equals `0`, no
//! tolerance makes two different values equal, and a NaN is no JSON number.
//! `=`, `<` and `>` are then a trichotomy and `=` is transitive, which no
//! index, mirror or binary search could honour with a tolerance.

use std::collections::HashMap;

use serde_json::{json, Value};
use velesdb_core::filter::{Condition, Filter};
use velesdb_core::{velesql::Parser, Database, Point};

use super::helpers::{create_test_db, execute_sql};

/// Numbers whose comparisons a rounding or a tolerance gets wrong.
fn edge_numbers() -> Vec<Value> {
    vec![
        json!(-0.0),
        json!(0),
        json!(0.0),
        json!(0.000_000_000_000_000_01),
        json!(-0.000_000_000_000_000_01),
        json!(0.1),
        json!(0.3),
        json!(0.300_000_000_000_000_04),
        json!(0.999_999_999_999_999_9),
        json!(1),
        json!(1.0),
        json!(9_007_199_254_740_991_u64),
        json!(9_007_199_254_740_992_u64),
        json!(9_007_199_254_740_992.0),
        json!(9_007_199_254_740_993_u64),
        json!(i64::MIN),
        json!(-9_223_372_036_854_775_808.0),
        json!(i64::MAX),
        json!(9_223_372_036_854_775_808_u64),
        json!(9_223_372_036_854_775_808.0),
        json!(u64::MAX),
        json!(18_446_744_073_709_551_616.0),
        json!(-1e300),
        json!(1e300),
    ]
}

/// Whether the filter `x <op> literal` keeps a point whose `x` is `value`.
fn holds(value: &Value, op: fn(String, Value) -> Condition, literal: &Value) -> bool {
    Filter::new(op("x".to_string(), literal.clone())).matches_point(1, Some(&json!({ "x": value })))
}

fn eq(field: String, value: Value) -> Condition {
    Condition::Eq { field, value }
}

fn lt(field: String, value: Value) -> Condition {
    Condition::Lt { field, value }
}

fn gt(field: String, value: Value) -> Condition {
    Condition::Gt { field, value }
}

fn neq(field: String, value: Value) -> Condition {
    Condition::Neq { field, value }
}

#[test]
fn test_equal_less_and_greater_are_a_trichotomy_over_every_pair_of_numbers() {
    let numbers = edge_numbers();
    for a in &numbers {
        for b in &numbers {
            let verdicts = [holds(a, lt, b), holds(a, eq, b), holds(a, gt, b)];
            assert_eq!(
                verdicts.iter().filter(|v| **v).count(),
                1,
                "{a} against {b}: <, =, > gave {verdicts:?}"
            );
            assert_eq!(holds(a, neq, b), !holds(a, eq, b), "{a} != {b}");
            assert_eq!(holds(a, lt, b), holds(b, gt, a), "{a} < {b} is {b} > {a}");
        }
    }
}

#[test]
fn test_equality_is_transitive_across_integers_and_floats() {
    let numbers = edge_numbers();
    for a in &numbers {
        for b in &numbers {
            for c in &numbers {
                if holds(a, eq, b) && holds(b, eq, c) {
                    assert!(holds(a, eq, c), "{a} = {b} and {b} = {c}, yet {a} != {c}");
                }
            }
        }
    }
}

#[test]
fn test_a_nan_is_no_json_number_and_matches_no_number() {
    // A NaN cannot be a JSON number; written as a literal it is `null`.
    assert!(serde_json::Number::from_f64(f64::NAN).is_none());
    let nan = json!(f64::NAN);
    for a in &edge_numbers() {
        for op in [eq, lt, gt] {
            assert!(!holds(a, op, &nan), "{a} against NaN");
        }
    }
}

#[test]
fn test_a_tolerance_no_longer_makes_two_numbers_equal() {
    // 7.1.0 took `0.1 + 0.2` (0.30000000000000004) for `0.3`.
    assert!(!holds(&json!(0.300_000_000_000_000_04), eq, &json!(0.3)));
    assert!(holds(&json!(0.3), eq, &json!(0.3)));
    assert!(holds(&json!(-0.0), eq, &json!(0)));
    assert!(holds(&json!(3), eq, &json!(3.0)));
}

fn match_count(db: &Database, sql: &str) -> usize {
    let query = Parser::parse(sql).expect("test: parse");
    let params = HashMap::from([("_collection".to_string(), json!("g"))]);
    db.execute_query(&query, &params)
        .expect("test: match")
        .len()
}

#[test]
fn test_match_compares_a_property_with_a_literal_exactly() {
    let (_dir, db) = create_test_db();
    execute_sql(
        &db,
        "CREATE COLLECTION g (dimension = 2, metric = 'cosine')",
    )
    .expect("test: create g");
    db.get_vector_collection("g")
        .expect("test: get g")
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0],
                Some(json!({"_labels": ["Doc"], "x": 3.5})),
            ),
            Point::new(
                2,
                vec![1.0, 0.0],
                Some(json!({"_labels": ["Doc"], "x": 3.0})),
            ),
            Point::new(
                3,
                vec![1.0, 0.0],
                Some(json!({"_labels": ["Doc"], "x": 9_007_199_254_740_993_u64})),
            ),
        ])
        .expect("test: seed g");

    let count =
        |predicate: &str| match_count(&db, &format!("MATCH (d:Doc) WHERE {predicate} RETURN d"));
    assert_eq!(
        count("d.x > 3"),
        2,
        "3.5 and 2^53 + 1 are above the integer 3"
    );
    assert_eq!(count("d.x = 3"), 1, "3.0 is the integer 3");
    assert_eq!(count("d.x = 3.0005"), 0, "no tolerance: 3.0 is not 3.0005");
    assert_eq!(count("d.x < 4"), 2, "3.5 and 3.0 are below the integer 4");
    assert_eq!(
        count("d.x > 9007199254740992.0"),
        1,
        "2^53 + 1 is above the float 2^53"
    );
}

#[test]
fn test_an_aggregate_counts_every_point_of_a_metadata_collection() {
    let (_dir, db) = create_test_db();
    execute_sql(&db, "CREATE METADATA COLLECTION meta").expect("test: create meta");
    db.get_metadata_collection("meta")
        .expect("test: get meta")
        .upsert(vec![
            Point::metadata_only(1, json!({"k": "a"})),
            Point::metadata_only(2, json!({"k": "b"})),
        ])
        .expect("test: seed meta");
    let aggregate = |sql: &str| {
        let query = Parser::parse(sql).expect("test: parse");
        db.execute_aggregate(&query, &HashMap::new())
            .expect("test: aggregate")
    };

    assert_eq!(
        aggregate("SELECT COUNT(*) AS n FROM meta").get("n"),
        Some(&json!(2))
    );
    assert_eq!(
        aggregate("SELECT COUNT(*) AS n FROM meta WHERE id = 1").get("n"),
        Some(&json!(1))
    );
    assert_eq!(
        aggregate("SELECT k, COUNT(*) AS n FROM meta GROUP BY k"),
        json!([{"k": "a", "n": 1}, {"k": "b", "n": 1}])
    );
}
