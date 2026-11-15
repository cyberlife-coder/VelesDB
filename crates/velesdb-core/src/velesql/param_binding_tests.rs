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

#[test]
fn a_missing_or_composite_parameter_is_refused() {
    let missing = bind_value(&param(), &HashMap::new()).expect_err("test: missing");
    assert!(
        missing.to_string().contains("Missing parameter: $p"),
        "{missing}"
    );
    for composite in [json!([1]), json!({"a": 1})] {
        let err = bind_value(&param(), &params(composite)).expect_err("test: composite");
        assert!(
            err.to_string()
                .contains("Unsupported parameter type for $p"),
            "{err}"
        );
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
    assert!(err.to_string().contains("Missing parameter: $q"), "{err}");
}
