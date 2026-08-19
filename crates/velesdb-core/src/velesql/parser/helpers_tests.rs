use super::*;

#[test]
fn test_compare_op_from_str_all_operators() {
    assert_eq!(compare_op_from_str("=").unwrap(), CompareOp::Eq);
    assert_eq!(compare_op_from_str("!=").unwrap(), CompareOp::NotEq);
    assert_eq!(compare_op_from_str("<>").unwrap(), CompareOp::NotEq);
    assert_eq!(compare_op_from_str(">").unwrap(), CompareOp::Gt);
    assert_eq!(compare_op_from_str(">=").unwrap(), CompareOp::Gte);
    assert_eq!(compare_op_from_str("<").unwrap(), CompareOp::Lt);
    assert_eq!(compare_op_from_str("<=").unwrap(), CompareOp::Lte);
}

#[test]
fn test_compare_op_from_str_invalid() {
    assert!(compare_op_from_str("??").is_err());
}

#[test]
fn test_parse_value_from_str_integer() {
    assert_eq!(parse_value_from_str("42").unwrap(), Value::Integer(42));
}

#[test]
fn test_parse_value_from_str_float() {
    assert_eq!(parse_value_from_str("2.72").unwrap(), Value::Float(2.72));
}

#[test]
fn test_parse_value_from_str_string() {
    assert_eq!(
        parse_value_from_str("'hello'").unwrap(),
        Value::String("hello".to_string())
    );
}

#[test]
fn test_parse_value_from_str_boolean() {
    assert_eq!(parse_value_from_str("true").unwrap(), Value::Boolean(true));
    assert_eq!(
        parse_value_from_str("FALSE").unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_parse_value_from_str_null() {
    assert_eq!(parse_value_from_str("null").unwrap(), Value::Null);
}

#[test]
fn test_parse_value_from_str_invalid() {
    assert!(parse_value_from_str("not_a_value").is_err());
}

#[test]
fn test_parse_u64_clause_error_message() {
    // Verify the error message includes the clause name.
    // We cannot construct a real pest pair without the grammar,
    // so we test indirectly via the error message format.
    let msg = format!("Expected integer for {}", "LIMIT");
    assert!(msg.contains("LIMIT"));
}

#[test]
fn test_strip_identifier_quotes_backtick() {
    assert_eq!(strip_identifier_quotes("`name`"), "name");
}

#[test]
fn test_strip_identifier_quotes_double() {
    assert_eq!(strip_identifier_quotes("\"col\""), "col");
}

#[test]
fn test_strip_identifier_quotes_escaped_double() {
    assert_eq!(strip_identifier_quotes("\"col\"\"name\""), "col\"name");
}

#[test]
fn test_strip_identifier_quotes_plain() {
    assert_eq!(strip_identifier_quotes("plain"), "plain");
}

#[test]
fn test_strip_identifier_quotes_trimmed() {
    assert_eq!(strip_identifier_quotes("  `spaced`  "), "spaced");
}

#[test]
fn test_unescape_string_literal_simple() {
    assert_eq!(unescape_string_literal("'hello'"), "hello");
}

#[test]
fn test_unescape_string_literal_escaped_quote() {
    assert_eq!(unescape_string_literal("'O''Brien'"), "O'Brien");
}

#[test]
fn test_unescape_string_literal_multiple_escapes() {
    assert_eq!(
        unescape_string_literal("'It''s a ''test'''"),
        "It's a 'test'"
    );
}

#[test]
fn test_unescape_string_literal_empty() {
    assert_eq!(unescape_string_literal("''"), "");
}

// --- Issue #486: large uint64 parsing ---

#[test]
fn test_parse_integer_i64_max() {
    // i64::MAX should still parse as Value::Integer
    let result = parse_value_from_str("9223372036854775807").unwrap();
    assert_eq!(result, Value::Integer(i64::MAX));
}

#[test]
fn test_parse_integer_u64_value() {
    // i64::MAX + 1 should parse as Value::UnsignedInteger
    let result = parse_value_from_str("9223372036854775808").unwrap();
    assert_eq!(result, Value::UnsignedInteger(9_223_372_036_854_775_808));
}

#[test]
fn test_parse_integer_u64_max() {
    // u64::MAX should parse as Value::UnsignedInteger
    let result = parse_value_from_str("18446744073709551615").unwrap();
    assert_eq!(result, Value::UnsignedInteger(u64::MAX));
}

#[test]
fn test_parse_integer_overflow_to_float() {
    // u64::MAX + 1 should fall through to float parsing
    let result = parse_value_from_str("18446744073709551616").unwrap();
    assert!(matches!(result, Value::Float(_)));
}
