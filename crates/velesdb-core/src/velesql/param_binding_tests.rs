use super::*;

use serde_json::json;

use crate::velesql::Parser;

fn params(value: serde_json::Value) -> HashMap<String, serde_json::Value> {
    HashMap::from([("p".to_string(), value)])
}

fn param() -> Value {
    Value::Parameter("p".to_string())
}

#[test]
fn a_number_binds_as_the_narrowest_type_that_holds_it() {
    assert_eq!(
        bind_value(&param(), &params(json!(-3))).ok(),
        Some(Value::Integer(-3))
    );
    assert_eq!(
        bind_value(&param(), &params(json!(u64::MAX))).ok(),
        Some(Value::UnsignedInteger(u64::MAX))
    );
    assert_eq!(
        bind_value(&param(), &params(json!(1.5))).ok(),
        Some(Value::Float(1.5))
    );
}

#[test]
fn a_string_boolean_or_null_binds_as_itself() {
    assert_eq!(
        bind_value(&param(), &params(json!("a"))).ok(),
        Some(Value::String("a".to_string()))
    );
    assert_eq!(
        bind_value(&param(), &params(json!(true))).ok(),
        Some(Value::Boolean(true))
    );
    assert_eq!(
        bind_value(&param(), &params(json!(null))).ok(),
        Some(Value::Null)
    );
}

/// A missing parameter and one bound to an array or object are both
/// parameter errors (`VELES-038`) naming `$p`, never a generic query error.
#[test]
fn a_missing_or_composite_parameter_is_refused() {
    let missing = bind_value(&param(), &HashMap::new()).expect_err("test: missing");
    assert!(
        matches!(&missing, Error::QueryParameter { name, .. } if name == "p"),
        "{missing}"
    );
    assert!(missing.to_string().contains("$p"), "{missing}");
    for composite in [json!([1]), json!({"a": 1})] {
        let err = bind_value(&param(), &params(composite)).expect_err("test: composite");
        assert!(
            matches!(&err, Error::QueryParameter { name, .. } if name == "p"),
            "{err}"
        );
        assert_eq!(err.code(), "VELES-038", "{err}");
    }
}

#[test]
fn a_literal_is_returned_unchanged() {
    let literal = Value::String("x".to_string());
    assert_eq!(bind_value(&literal, &HashMap::new()).ok(), Some(literal));
}

/// Every scalar operand is bound, through every combinator.
#[test]
fn a_condition_binds_its_operands_through_every_combinator() {
    let parsed = Parser::parse(
        "SELECT * FROM t WHERE NOT (a = $p AND (b IN ($p, 2) OR c BETWEEN $p AND 9)) \
         AND tags CONTAINS ANY ($p) LIMIT 1",
    )
    .expect("test: parse");
    let cond = parsed.select.where_clause.expect("test: WHERE");
    let bound = bind_condition(&cond, &params(json!(7))).expect("test: bind");
    let rendered = format!("{bound:?}");
    assert!(!rendered.contains("Parameter"), "{rendered}");
    assert_eq!(rendered.matches("Integer(7)").count(), 4, "{rendered}");
}

#[test]
fn a_condition_fails_on_its_first_unbound_operand() {
    let parsed =
        Parser::parse("SELECT * FROM t WHERE a = 1 OR b = $q LIMIT 1").expect("test: parse");
    let cond = parsed.select.where_clause.expect("test: WHERE");
    let err = bind_condition(&cond, &HashMap::new()).expect_err("test: unbound");
    assert!(
        matches!(&err, Error::QueryParameter { name, .. } if name == "q"),
        "{err}"
    );
}

#[test]
fn a_vector_literal_binds_as_itself_and_a_parameter_as_its_numbers() {
    let literal = VectorExpr::Literal(vec![0.5, -1.0]);
    assert_eq!(
        bind_vector(&literal, &HashMap::new()).ok(),
        Some(vec![0.5, -1.0])
    );
    let bound = VectorExpr::Parameter("p".to_string());
    assert_eq!(
        bind_vector(&bound, &params(json!([1, 0.25]))).ok(),
        Some(vec![1.0, 0.25])
    );
}

/// A vector parameter that is missing, or bound to anything but an array of
/// numbers in `f32` range, is a parameter error (`VELES-038`) naming `$p`.
#[test]
fn a_vector_parameter_missing_or_of_the_wrong_type_is_refused() {
    let expr = VectorExpr::Parameter("p".to_string());
    let missing = bind_vector(&expr, &HashMap::new()).expect_err("test: missing");
    assert_eq!(
        missing.to_string(),
        Error::unbound_parameter("p").to_string()
    );
    for bound in [
        json!("not a vector"),
        json!({"x": 1.0}),
        json!([1.0, "x"]),
        json!([1e300]),
    ] {
        let err = bind_vector(&expr, &params(bound)).expect_err("test: wrong type");
        assert!(
            matches!(&err, Error::QueryParameter { name, .. } if name == "p"),
            "{err}"
        );
        assert_eq!(err.code(), "VELES-038", "{err}");
    }
}

/// The guard behind `vector_element` handling only finite numbers: a
/// `serde_json::Number` cannot hold a non-finite value (`from_f64` refuses
/// one, and no workspace crate enables `arbitrary_precision`), and a JSON
/// number out of `f64` range does not parse, so a bound element `as_f64`
/// reads is always finite.
#[test]
fn a_json_number_is_always_finite() {
    for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(serde_json::Number::from_f64(non_finite).is_none());
    }
    assert!(serde_json::from_str::<serde_json::Value>("1e400").is_err());
}

#[test]
fn a_dml_value_binds_a_literal_as_json_and_a_parameter_as_given() {
    assert_eq!(
        bind_dml_value(&Value::Integer(7), &params(json!(null))).ok(),
        Some(json!(7))
    );
    assert_eq!(
        bind_dml_value(&Value::String("a".into()), &params(json!(null))).ok(),
        Some(json!("a"))
    );
    // A DML cell stores the caller's JSON whole: an object is a payload value.
    assert_eq!(
        bind_dml_value(&param(), &params(json!({"k": [1, 2]}))).ok(),
        Some(json!({"k": [1, 2]}))
    );
}

#[test]
fn a_dml_value_unbound_or_a_subquery_is_core_s_error() {
    let unbound = bind_dml_value(&Value::Parameter("q".into()), &params(json!(1)))
        .expect_err("test: $q is unbound");
    assert_eq!(unbound.code(), "VELES-038");
    let query = Parser::parse("SELECT * FROM t WHERE a = (SELECT MAX(b) FROM u)")
        .expect("test: a scalar subquery parses");
    let subquery = first_comparison_value(&query);
    let err = bind_dml_value(&subquery, &params(json!(1))).expect_err("test: subquery");
    assert_eq!(err.code(), "VELES-010");
    assert_eq!(
        err.to_string(),
        "[VELES-010] Query error: Subquery values are not supported in INSERT/UPDATE"
    );
}

#[test]
fn a_dml_vector_binds_a_parameter_and_refuses_any_literal() {
    assert_eq!(
        bind_dml_vector(&param(), &params(json!([1.0, 2.0]))).ok(),
        Some(vec![1.0, 2.0])
    );
    let unbound = bind_dml_vector(&Value::Parameter("q".into()), &params(json!([1.0])))
        .expect_err("test: $q is unbound");
    assert_eq!(unbound.code(), "VELES-038");
    for literal in [Value::Null, Value::String("x".into()), Value::Integer(1)] {
        let err = bind_dml_vector(&literal, &params(json!(null))).expect_err("test: literal");
        assert_eq!(
            err.to_string(),
            "[VELES-010] Query error: 'vector' must be an array of numbers"
        );
    }
}

/// The right operand of the first comparison in `query`'s WHERE.
fn first_comparison_value(query: &crate::velesql::Query) -> Value {
    match query.select.where_clause.as_ref() {
        Some(Condition::Comparison(c)) => c.value.clone(),
        other => panic!("test: expected a comparison, got {other:?}"),
    }
}
